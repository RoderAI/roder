use roder_api::interactive::{InteractiveRegion, RegionId};

#[derive(Debug, Clone, Default)]
pub struct RegionFrame {
    regions: Vec<FrameRegion>,
    next_order: usize,
}

#[derive(Debug, Clone)]
struct FrameRegion {
    region: InteractiveRegion,
    order: usize,
}

impl RegionFrame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, region: InteractiveRegion) {
        let order = self.next_order;
        self.next_order += 1;
        self.regions.push(FrameRegion { region, order });
    }

    pub fn hit_test(&self, x: u16, y: u16) -> Option<&InteractiveRegion> {
        self.regions
            .iter()
            .filter(|entry| entry.region.rect.contains(x, y))
            .max_by_key(|entry| (entry.region.z, entry.order))
            .map(|entry| &entry.region)
    }

    pub fn get(&self, id: &RegionId) -> Option<&InteractiveRegion> {
        self.regions
            .iter()
            .find(|entry| &entry.region.id == id)
            .map(|entry| &entry.region)
    }

    pub fn iter(&self) -> impl Iterator<Item = &InteractiveRegion> {
        self.regions.iter().map(|entry| &entry.region)
    }
}

#[cfg(test)]
mod tests {
    use roder_api::interactive::{HoverCursor, RegionKind, RegionRect};

    use super::*;

    fn region(id: &str, rect: RegionRect, z: i16) -> InteractiveRegion {
        InteractiveRegion {
            id: id.to_string(),
            rect,
            z,
            kind: RegionKind::Composer,
            hover_cursor: HoverCursor::Default,
            keyboard_binding: None,
        }
    }

    #[test]
    fn hit_test_prefers_highest_z_then_most_recent_region() {
        let mut frame = RegionFrame::new();
        let rect = RegionRect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        frame.push(region("first", rect, 1));
        frame.push(region("second", rect, 2));
        frame.push(region("third", rect, 2));

        assert_eq!(
            frame.hit_test(2, 2).map(|region| region.id.as_str()),
            Some("third")
        );
        assert!(frame.hit_test(8, 8).is_none());
    }
}
