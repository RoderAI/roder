use std::io::{IsTerminal, Write};

use super::{ImagePreviews, Rect};

impl ImagePreviews {
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        if !visible {
            for image in self
                .entries
                .values()
                .flatten()
                .filter_map(|entry| entry.image.as_ref())
            {
                image.suspend();
            }
            let areas = std::mem::take(self.last_sixel_areas.get_mut());
            if !areas.is_empty() && std::io::stdout().is_terminal() {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(sixel_clear_sequence(&areas).as_bytes());
                let _ = stdout.flush();
            }
        } else {
            // Deleted Kitty data cannot be redisplayed with the old transmitted flag.
            for entry in self.entries.values_mut().flatten() {
                entry.image = None;
                entry.width = 0;
            }
        }
        true
    }
}

fn sixel_clear_sequence(areas: &[Rect]) -> String {
    let mut command = String::from("\x1b7");
    for area in areas {
        for y in area.y..area.bottom() {
            command.push_str(&format!(
                "\x1b[{};{}H\x1b[{}X",
                u32::from(y) + 1,
                u32::from(area.x) + 1,
                area.width,
            ));
        }
    }
    command.push_str("\x1b8");
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::media::image::{self, TerminalImages};
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};
    use ratatui_image::{FontSize, picker::ProtocolType};

    #[test]
    fn sixel_cleanup_preserves_cursor_and_erases_only_owned_cells() {
        assert_eq!(
            sixel_clear_sequence(&[Rect::new(2, 4, 10, 2)]),
            "\x1b7\x1b[5;3H\x1b[10X\x1b[6;3H\x1b[10X\x1b8",
        );
    }

    #[test]
    fn overlays_suspend_all_pixels_then_rebuild_protocols_before_resuming() {
        let path =
            std::env::temp_dir().join(format!("roder-preview-overlay-{}.png", std::process::id()));
        ::image::RgbaImage::from_pixel(16, 128, ::image::Rgba([30, 40, 50, 255]))
            .save(&path)
            .unwrap();
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel] {
            let terminal_images = TerminalImages {
                protocol,
                cell: FontSize::new(8, 16),
                tmux: false,
            };
            let mut previews = ImagePreviews {
                terminal: Some(terminal_images),
                ..ImagePreviews::default()
            };
            previews.attach(0, [path.clone()]);
            previews.entries.get_mut(&0).unwrap()[0].image =
                Some(image::load(&path, 20, terminal_images).unwrap());
            let rows = (0..9).map(|row| (row, 0)).collect::<Vec<_>>();
            let area = Rect::new(0, 0, 20, 5);
            let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
            terminal
                .draw(|frame| previews.render(frame, area, 3, &rows))
                .unwrap();
            assert!(previews.set_visible(false));
            assert!(!previews.set_visible(false));
            assert!(previews.last_sixel_areas.borrow().is_empty());
            terminal
                .draw(|frame| {
                    previews.render(frame, area, 3, &rows);
                    frame.render_widget(Paragraph::new("permission dialog"), area);
                })
                .unwrap();
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| !cell.symbol().contains('\x1b'))
            );
            assert!(previews.set_visible(true));
            assert!(previews.entries[&0][0].image.is_none());
            assert_eq!(previews.entries[&0][0].width, 0);
        }
        std::fs::remove_file(path).unwrap();
    }
}
