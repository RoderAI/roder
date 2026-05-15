use ratatui::layout::Rect;
use roder_api::interactive::{InteractiveRegion, RegionId, RegionRect};

#[derive(Debug, Clone, Default)]
pub struct RegionFrame {
    regions: Vec<RenderedRegion>,
}

#[derive(Debug, Clone)]
struct RenderedRegion {
    order: usize,
    region: InteractiveRegion,
}

impl RegionFrame {
    pub fn builder() -> RegionFrameBuilder {
        RegionFrameBuilder::default()
    }

    pub fn hit_test(&self, x: u16, y: u16) -> Option<&InteractiveRegion> {
        self.regions
            .iter()
            .rev()
            .find(|rendered| rendered.region.rect.contains(x, y))
            .map(|rendered| &rendered.region)
    }

    pub fn get(&self, id: &str) -> Option<&InteractiveRegion> {
        self.regions
            .iter()
            .find(|rendered| rendered.region.id == id)
            .map(|rendered| &rendered.region)
    }

    pub fn region_ids(&self) -> impl Iterator<Item = &RegionId> {
        self.regions.iter().map(|rendered| &rendered.region.id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegionFrameBuilder {
    regions: Vec<RenderedRegion>,
}

impl RegionFrameBuilder {
    pub fn push(&mut self, region: InteractiveRegion) {
        let order = self.regions.len();
        self.regions.push(RenderedRegion { order, region });
    }

    pub fn build(mut self) -> RegionFrame {
        self.regions
            .sort_by_key(|rendered| (rendered.region.z, rendered.order));
        RegionFrame {
            regions: self.regions,
        }
    }
}

pub fn region_rect_from_ratatui(rect: Rect) -> RegionRect {
    RegionRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::{HoverCursor, RegionKind};

    use super::*;

    #[test]
    fn hit_test_prefers_highest_z_then_latest_region() {
        let mut builder = RegionFrame::builder();
        builder.push(region("low", 0));
        builder.push(region("first-high", 1));
        builder.push(region("latest-high", 1));
        let frame = builder.build();

        assert_eq!(frame.hit_test(1, 1).unwrap().id, "latest-high");
        assert!(frame.hit_test(99, 99).is_none());
    }

    fn region(id: &str, z: i16) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            z,
            kind: RegionKind::Composer,
            hover_cursor: HoverCursor::Default,
            keyboard_binding: None,
        }
    }
}
