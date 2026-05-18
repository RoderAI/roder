#![allow(dead_code)]

use std::collections::HashMap;

use ratatui::text::Line;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct VirtualViewport {
    pub scroll_offset: usize,
    pub height: usize,
    pub overscan_rows: usize,
}

impl VirtualViewport {
    pub(super) fn visible_bounds(&self) -> (usize, usize) {
        (
            self.scroll_offset,
            self.scroll_offset.saturating_add(self.height),
        )
    }

    fn overscan_bounds(&self) -> (usize, usize) {
        let top = self.scroll_offset.saturating_sub(self.overscan_rows);
        let bottom = self
            .scroll_offset
            .saturating_add(self.height)
            .saturating_add(self.overscan_rows);
        (top, bottom)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct VirtualTimelineItem {
    pub item_index: Option<usize>,
    pub height: usize,
}

impl VirtualTimelineItem {
    pub(super) fn item(item_index: usize, height: usize) -> Self {
        Self {
            item_index: Some(item_index),
            height,
        }
    }

    pub(super) fn padding(height: usize) -> Self {
        Self {
            item_index: None,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct VirtualItemLayout {
    pub layout_index: usize,
    pub item_index: Option<usize>,
    pub start_row: usize,
    pub height: usize,
}

impl VirtualItemLayout {
    pub(super) fn end_row(&self) -> usize {
        self.start_row.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct VirtualVisibleRange {
    pub first_layout_index: usize,
    pub last_layout_index: usize,
    pub render_start_row: usize,
    pub render_end_row: usize,
    pub viewport_top: usize,
    pub viewport_bottom: usize,
    pub total_height: usize,
}

impl VirtualVisibleRange {
    pub(super) fn layout_indices(&self) -> std::ops::RangeInclusive<usize> {
        self.first_layout_index..=self.last_layout_index
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct VirtualHitRow {
    pub absolute_row: usize,
    pub viewport_row: u16,
    pub item_index: usize,
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub(super) struct VirtualRenderKey {
    item_index: usize,
    width: u16,
    theme_revision: u64,
    selected: bool,
    expanded: bool,
    generation: u64,
    animation_frame: Option<u64>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct VirtualRenderInput {
    pub item_index: usize,
    pub width: u16,
    pub theme_revision: u64,
    pub selected: bool,
    pub expanded: bool,
    pub generation: u64,
    pub animation_sensitive: bool,
    pub animation_frame: u64,
}

impl VirtualRenderInput {
    fn cache_key(self) -> VirtualRenderKey {
        VirtualRenderKey {
            item_index: self.item_index,
            width: self.width,
            theme_revision: self.theme_revision,
            selected: self.selected,
            expanded: self.expanded,
            generation: self.generation,
            animation_frame: self.animation_sensitive.then_some(self.animation_frame),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct VirtualCachedRender {
    pub lines: Vec<Line<'static>>,
    pub visual_height: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct VirtualRenderLookup {
    pub render: VirtualCachedRender,
    pub reused: bool,
}

#[derive(Debug, Default)]
pub(super) struct VirtualRenderCache {
    entries: HashMap<VirtualRenderKey, VirtualCachedRender>,
}

impl VirtualRenderCache {
    pub(super) fn get_or_render(
        &mut self,
        input: VirtualRenderInput,
        render: impl FnOnce() -> Vec<Line<'static>>,
    ) -> VirtualRenderLookup {
        let key = input.cache_key();
        if let Some(render) = self.entries.get(&key) {
            return VirtualRenderLookup {
                render: render.clone(),
                reused: true,
            };
        }

        let lines = render();
        let visual_height = visual_height_for_lines(&lines, input.width);
        let cached = VirtualCachedRender {
            lines,
            visual_height,
        };
        self.entries.insert(key, cached.clone());
        VirtualRenderLookup {
            render: cached,
            reused: false,
        }
    }

    pub(super) fn invalidate_item(&mut self, item_index: usize) {
        self.entries.retain(|key, _| key.item_index != item_index);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(super) struct VirtualTimelineLayout {
    items: Vec<VirtualItemLayout>,
    prefix_rows: Vec<usize>,
    total_height: usize,
}

impl VirtualTimelineLayout {
    pub(super) fn from_items(items: impl IntoIterator<Item = VirtualTimelineItem>) -> Self {
        let mut next_row = 0usize;
        let mut prefix_rows = Vec::new();
        let items = items
            .into_iter()
            .enumerate()
            .filter_map(|(layout_index, item)| {
                if item.height == 0 {
                    return None;
                }
                let start_row = next_row;
                prefix_rows.push(start_row);
                next_row = next_row.saturating_add(item.height);
                Some(VirtualItemLayout {
                    layout_index,
                    item_index: item.item_index,
                    start_row,
                    height: item.height,
                })
            })
            .collect::<Vec<_>>();
        Self {
            items,
            prefix_rows,
            total_height: next_row,
        }
    }

    pub(super) fn total_height(&self) -> usize {
        self.total_height
    }

    pub(super) fn max_scroll(&self, viewport_height: usize) -> usize {
        self.total_height.saturating_sub(viewport_height)
    }

    pub(super) fn item_at_row(&self, row: usize) -> Option<&VirtualItemLayout> {
        if row >= self.total_height {
            return None;
        }
        let index = self
            .prefix_rows
            .partition_point(|start_row| *start_row <= row)
            .saturating_sub(1);
        self.items.get(index)
    }

    pub(super) fn item(&self, layout_index: usize) -> Option<&VirtualItemLayout> {
        self.items
            .iter()
            .find(|item| item.layout_index == layout_index)
    }

    pub(super) fn visible_range(&self, viewport: VirtualViewport) -> Option<VirtualVisibleRange> {
        if self.items.is_empty() || viewport.height == 0 {
            return None;
        }

        let (viewport_top, viewport_bottom) = viewport.visible_bounds();
        let (render_top, render_bottom) = viewport.overscan_bounds();
        let render_bottom = render_bottom.min(self.total_height);
        let first = self
            .items
            .iter()
            .find(|item| item.end_row() > render_top && item.start_row < render_bottom)?;
        let last = self
            .items
            .iter()
            .rev()
            .find(|item| item.end_row() > render_top && item.start_row < render_bottom)?;

        Some(VirtualVisibleRange {
            first_layout_index: first.layout_index,
            last_layout_index: last.layout_index,
            render_start_row: first.start_row,
            render_end_row: last.end_row(),
            viewport_top,
            viewport_bottom,
            total_height: self.total_height,
        })
    }

    pub(super) fn visible_hit_rows(
        &self,
        viewport: VirtualViewport,
        viewport_y: u16,
    ) -> Vec<VirtualHitRow> {
        if viewport.height == 0 {
            return Vec::new();
        }

        let (top, bottom) = viewport.visible_bounds();
        self.items
            .iter()
            .filter_map(|item| {
                let item_index = item.item_index?;
                if item.end_row() <= top || item.start_row >= bottom {
                    return None;
                }
                let start = item.start_row.max(top);
                let end = item.end_row().min(bottom);
                Some((start..end).map(move |absolute_row| VirtualHitRow {
                    absolute_row,
                    viewport_row: viewport_y + (absolute_row - top) as u16,
                    item_index,
                }))
            })
            .flatten()
            .collect()
    }
}

fn visual_height_for_lines(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width).max(1);
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_virtualization_layout_maps_variable_height_rows() {
        let layout = VirtualTimelineLayout::from_items([
            VirtualTimelineItem::item(10, 2),
            VirtualTimelineItem::item(11, 5),
            VirtualTimelineItem::item(12, 1),
        ]);

        assert_eq!(layout.total_height(), 8);
        assert_eq!(
            layout.item_at_row(0).map(|item| item.item_index),
            Some(Some(10))
        );
        assert_eq!(
            layout.item_at_row(1).map(|item| item.item_index),
            Some(Some(10))
        );
        assert_eq!(
            layout.item_at_row(2).map(|item| item.item_index),
            Some(Some(11))
        );
        assert_eq!(
            layout.item_at_row(6).map(|item| item.item_index),
            Some(Some(11))
        );
        assert_eq!(
            layout.item_at_row(7).map(|item| item.item_index),
            Some(Some(12))
        );
        assert_eq!(layout.item_at_row(8), None);
    }

    #[test]
    fn timeline_virtualization_layout_overscans_nearby_items_only() {
        let layout = VirtualTimelineLayout::from_items([
            VirtualTimelineItem::item(0, 3),
            VirtualTimelineItem::item(1, 4),
            VirtualTimelineItem::item(2, 6),
            VirtualTimelineItem::item(3, 3),
        ]);

        let visible = layout
            .visible_range(VirtualViewport {
                scroll_offset: 7,
                height: 3,
                overscan_rows: 2,
            })
            .expect("visible range");

        assert_eq!(visible.first_layout_index, 1);
        assert_eq!(visible.last_layout_index, 2);
        assert_eq!(visible.render_start_row, 3);
        assert_eq!(visible.render_end_row, 13);
        assert_eq!(visible.viewport_top, 7);
        assert_eq!(visible.viewport_bottom, 10);
    }

    #[test]
    fn timeline_virtualization_layout_counts_bottom_padding_in_height() {
        let layout = VirtualTimelineLayout::from_items([
            VirtualTimelineItem::item(0, 2),
            VirtualTimelineItem::padding(3),
        ]);

        assert_eq!(layout.total_height(), 5);
        assert_eq!(layout.max_scroll(2), 3);

        let range = layout
            .visible_range(VirtualViewport {
                scroll_offset: 3,
                height: 2,
                overscan_rows: 0,
            })
            .expect("padding remains visible");
        assert_eq!(range.first_layout_index, 1);
        assert_eq!(range.last_layout_index, 1);
        assert_eq!(layout.item(1).map(|item| item.item_index), Some(None));
    }

    #[test]
    fn timeline_virtualization_layout_maps_virtual_hit_rows() {
        let layout = VirtualTimelineLayout::from_items([
            VirtualTimelineItem::item(4, 2),
            VirtualTimelineItem::padding(2),
            VirtualTimelineItem::item(5, 3),
        ]);

        let hit_rows = layout.visible_hit_rows(
            VirtualViewport {
                scroll_offset: 1,
                height: 5,
                overscan_rows: 10,
            },
            10,
        );

        assert_eq!(
            hit_rows,
            vec![
                VirtualHitRow {
                    absolute_row: 1,
                    viewport_row: 10,
                    item_index: 4,
                },
                VirtualHitRow {
                    absolute_row: 4,
                    viewport_row: 13,
                    item_index: 5,
                },
                VirtualHitRow {
                    absolute_row: 5,
                    viewport_row: 14,
                    item_index: 5,
                },
            ]
        );
    }

    #[test]
    fn timeline_virtualization_cache_reuses_same_item_render() {
        let mut cache = VirtualRenderCache::default();
        let input = VirtualRenderInput {
            item_index: 7,
            width: 10,
            theme_revision: 1,
            selected: false,
            expanded: false,
            generation: 0,
            animation_sensitive: false,
            animation_frame: 0,
        };

        let first = cache.get_or_render(input, || vec![Line::raw("hello")]);
        let second = cache.get_or_render(input, || vec![Line::raw("different")]);

        assert!(!first.reused);
        assert!(second.reused);
        assert_eq!(second.render.lines, vec![Line::raw("hello")]);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn timeline_virtualization_cache_invalidates_width_expansion_selection_and_content() {
        let mut cache = VirtualRenderCache::default();
        let base = VirtualRenderInput {
            item_index: 1,
            width: 12,
            theme_revision: 1,
            selected: false,
            expanded: false,
            generation: 0,
            animation_sensitive: false,
            animation_frame: 0,
        };

        assert!(!cache.get_or_render(base, || vec![Line::raw("base")]).reused);
        assert!(
            !cache
                .get_or_render(VirtualRenderInput { width: 8, ..base }, || vec![Line::raw(
                    "width"
                )])
                .reused
        );
        assert!(
            !cache
                .get_or_render(
                    VirtualRenderInput {
                        expanded: true,
                        ..base
                    },
                    || vec![Line::raw("expanded")]
                )
                .reused
        );
        assert!(
            !cache
                .get_or_render(
                    VirtualRenderInput {
                        selected: true,
                        ..base
                    },
                    || vec![Line::raw("selected")]
                )
                .reused
        );
        assert!(
            !cache
                .get_or_render(
                    VirtualRenderInput {
                        generation: 1,
                        ..base
                    },
                    || vec![Line::raw("updated")]
                )
                .reused
        );
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn timeline_virtualization_cache_limits_animation_invalidation_to_sensitive_items() {
        let mut cache = VirtualRenderCache::default();
        let static_item = VirtualRenderInput {
            item_index: 1,
            width: 12,
            theme_revision: 1,
            selected: false,
            expanded: false,
            generation: 0,
            animation_sensitive: false,
            animation_frame: 0,
        };
        let animated_item = VirtualRenderInput {
            item_index: 2,
            animation_sensitive: true,
            ..static_item
        };

        assert!(
            !cache
                .get_or_render(static_item, || vec![Line::raw("static")])
                .reused
        );
        assert!(
            cache
                .get_or_render(
                    VirtualRenderInput {
                        animation_frame: 1,
                        ..static_item
                    },
                    || vec![Line::raw("static rerender")]
                )
                .reused
        );
        assert!(
            !cache
                .get_or_render(animated_item, || vec![Line::raw("frame 0")])
                .reused
        );
        assert!(
            !cache
                .get_or_render(
                    VirtualRenderInput {
                        animation_frame: 1,
                        ..animated_item
                    },
                    || vec![Line::raw("frame 1")]
                )
                .reused
        );
    }

    #[test]
    fn timeline_virtualization_cache_records_visual_height() {
        let mut cache = VirtualRenderCache::default();
        let lookup = cache.get_or_render(
            VirtualRenderInput {
                item_index: 3,
                width: 4,
                theme_revision: 1,
                selected: false,
                expanded: false,
                generation: 0,
                animation_sensitive: false,
                animation_frame: 0,
            },
            || vec![Line::raw("abcdef"), Line::raw("")],
        );

        assert_eq!(lookup.render.visual_height, 3);
    }
}
