//! Bounded, local-only decoding and terminal protocol selection. Runs on the preview worker.

use std::{
    fs::File,
    io::{Cursor, IsTerminal, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use image::{DynamicImage, ImageFormat, ImageReader};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Rect, Size},
    widgets::Widget,
};
use ratatui_image::{
    FontSize, Image,
    picker::ProtocolType,
    protocol::{Protocol, kitty::Kitty, sixel::Sixel},
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};

const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RASTER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
pub(super) const MAX_COLUMNS: u16 = 48;
pub(super) const MAX_ROWS: u16 = 20;
static NEXT_IMAGE_ID: AtomicU32 = AtomicU32::new(0x524F_0000);

#[derive(Clone, Copy, Debug)]
pub(super) struct TerminalImages {
    pub protocol: ProtocolType,
    pub cell: FontSize,
    pub tmux: bool,
}

impl TerminalImages {
    /// Do not consume stdin: the event reader owns terminal responses. Unknown terminals,
    /// multiplexers without explicit opt-in, and Sixel without measured cells use labels.
    pub fn detect() -> Option<Self> {
        if !std::io::stdout().is_terminal() {
            return None;
        }
        let cell = crossterm::terminal::window_size().ok().and_then(|size| {
            (size.columns > 0 && size.rows > 0 && size.width > 0 && size.height > 0)
                .then(|| FontSize::new(size.width / size.columns, size.height / size.rows))
                .filter(|size| size.width > 0 && size.height > 0)
        });
        Self::from_environment(
            &std::env::var("RODER_IMAGE_PROTOCOL").unwrap_or_default(),
            &std::env::var("TERM").unwrap_or_default(),
            &std::env::var("TERM_PROGRAM").unwrap_or_default(),
            std::env::var_os("TMUX").is_some(),
            cell,
        )
    }

    fn from_environment(
        requested: &str,
        term: &str,
        program: &str,
        tmux: bool,
        cell: Option<FontSize>,
    ) -> Option<Self> {
        let protocol = match requested {
            "none" | "off" => return None,
            "kitty" => ProtocolType::Kitty,
            "sixel" => ProtocolType::Sixel,
            "" | "auto" if !tmux => {
                if term == "xterm-kitty" || program.eq_ignore_ascii_case("ghostty") {
                    ProtocolType::Kitty
                } else if term.starts_with("foot") || term.contains("sixel") {
                    ProtocolType::Sixel
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // Sixel paints physical pixels; guessing the cell size risks painting over the composer.
        let cell = match protocol {
            ProtocolType::Sixel => cell?,
            _ => cell.unwrap_or(FontSize::new(8, 16)),
        };
        Some(Self {
            protocol,
            cell,
            tmux,
        })
    }
}

enum EncodedImage {
    Kitty(SlicedProtocol),
    // Encode exact cell-height strips on the worker. Generic Sixel band slicing rounds to
    // six-pixel bands and can overpaint a viewport clipped at both the top and the bottom.
    Sixel(Vec<Protocol>),
}

pub(super) struct LoadedImage {
    protocol: EncodedImage,
    pub size: Size,
    kitty_id: Option<u32>,
    tmux: bool,
    rendered: AtomicBool,
}

impl LoadedImage {
    pub fn is_sixel(&self) -> bool {
        matches!(self.protocol, EncodedImage::Sixel(_))
    }

    /// Hide owned Kitty placements outside drawing. Cached protocols must be rebuilt before
    /// reuse because the terminal data and the library's transmitted flag are now out of sync.
    pub fn suspend(&self) {
        let rendered = self.rendered.swap(false, Ordering::Relaxed);
        if !rendered || !std::io::stdout().is_terminal() {
            return;
        }
        if let Some(id) = self.kitty_id {
            let command = kitty_cleanup_command(id, self.tmux);
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(command.as_bytes());
            let _ = stdout.flush();
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, position: SignedPosition) {
        self.rendered.store(true, Ordering::Relaxed);
        self.render_into(area, position, frame.buffer_mut());
    }

    fn render_into(&self, area: Rect, position: SignedPosition, buffer: &mut Buffer) {
        if self.size.width > area.width.saturating_sub(position.x.max(0) as u16) {
            return;
        }
        match &self.protocol {
            EncodedImage::Kitty(protocol) => {
                SlicedImage::new(protocol, position).render(area, buffer);
            }
            EncodedImage::Sixel(rows) => {
                let skip = usize::from(position.y.unsigned_abs()) * usize::from(position.y < 0);
                let y = position.y.max(0) as u16;
                let available = usize::from(area.height.saturating_sub(y));
                for (row, protocol) in rows.iter().skip(skip).take(available).enumerate() {
                    let row_area = Rect::new(
                        area.x + position.x.max(0) as u16,
                        area.y + y + row as u16,
                        self.size.width,
                        1,
                    );
                    Image::new(protocol).render(row_area, buffer);
                }
            }
        }
    }
}

impl Drop for LoadedImage {
    fn drop(&mut self) {
        self.suspend();
    }
}

fn kitty_cleanup_command(id: u32, tmux: bool) -> String {
    // Never issue a global delete that could remove another application's images.
    let command = format!("\x1b_Ga=d,d=I,i={id},q=2;\x1b\\");
    if tmux {
        format!("\x1bPtmux;{}\x1b\\", command.replace('\x1b', "\x1b\x1b"))
    } else {
        command
    }
}

pub(super) fn load(
    path: &Path,
    width: u16,
    terminal: TerminalImages,
) -> Result<LoadedImage, String> {
    let image = decode(path)?;
    let size = preview_size(image.width(), image.height(), width, terminal.cell)
        .ok_or_else(|| "not enough space".to_string())?;
    let image = image.resize_exact(
        u32::from(size.width) * u32::from(terminal.cell.width),
        u32::from(size.height) * u32::from(terminal.cell.height),
        image::imageops::FilterType::Triangle,
    );
    let (protocol, kitty_id) = if terminal.protocol == ProtocolType::Kitty {
        let id = NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed);
        // Compression requires an explicit o=z capability response. Do not assume it from
        // terminal environment hints or race the event reader by probing stdin here.
        let kitty = Kitty::new(image, size, id, terminal.tmux, false)
            .map_err(|_| "terminal encoding failed".to_string())?;
        (EncodedImage::Kitty(SlicedProtocol::Kitty(kitty)), Some(id))
    } else {
        let mut rows = Vec::with_capacity(usize::from(size.height));
        for row in 0..size.height {
            let strip = image.crop_imm(
                0,
                u32::from(row) * u32::from(terminal.cell.height),
                image.width(),
                u32::from(terminal.cell.height),
            );
            let sixel = Sixel::new(strip, Size::new(size.width, 1), terminal.tmux)
                .map_err(|_| "terminal encoding failed".to_string())?;
            rows.push(Protocol::Sixel(sixel));
        }
        (EncodedImage::Sixel(rows), None)
    };
    Ok(LoadedImage {
        protocol,
        size,
        kitty_id,
        tmux: terminal.tmux,
        rendered: AtomicBool::new(false),
    })
}

pub(super) fn preview_size(width: u32, height: u32, columns: u16, cell: FontSize) -> Option<Size> {
    if cell.width == 0 || cell.height == 0 {
        return None;
    }
    let columns = columns
        .saturating_sub(2)
        .min(MAX_COLUMNS)
        .min(1024 / cell.width);
    let max_rows = MAX_ROWS.min(512 / cell.height);
    if width == 0 || height == 0 || columns == 0 || max_rows == 0 {
        return None;
    }
    let scale = (f64::from(columns) * f64::from(cell.width) / f64::from(width))
        .min(f64::from(max_rows) * f64::from(cell.height) / f64::from(height))
        .min(1.0);
    let cols = (f64::from(width) * scale / f64::from(cell.width)).ceil() as u16;
    let rows = (f64::from(height) * scale / f64::from(cell.height)).ceil() as u16;
    Some(Size::new(cols.clamp(1, columns), rows.clamp(1, max_rows)))
}

fn decode(path: &Path) -> Result<DynamicImage, String> {
    let metadata = path.metadata().map_err(|_| "file unavailable")?;
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err("image exceeds 32 MiB".into());
    }
    let file = File::open(path).map_err(|_| "file unavailable")?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "image read failed")?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err("image exceeds 32 MiB".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_RASTER_BYTES);
    let mut reader = ImageReader::new(Cursor::new(bytes.as_slice()))
        .with_guessed_format()
        .map_err(|_| "unrecognized image")?;
    reader.limits(limits.clone());
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return Err("preview supports PNG, JPEG and WebP".into());
    }
    let format = reader.format().ok_or("unrecognized image")?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "invalid or oversized image")?;
    if width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) * 4 > MAX_RASTER_BYTES
    {
        return Err("invalid or oversized image".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| "invalid or oversized image".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_tmux_terminals_use_metadata_without_explicit_opt_in() {
        assert!(TerminalImages::from_environment("", "xterm", "", false, None).is_none());
        assert!(TerminalImages::from_environment("", "xterm-kitty", "", true, None).is_none());
        assert!(TerminalImages::from_environment("none", "xterm-kitty", "", false, None).is_none());
        assert!(TerminalImages::from_environment("kitty", "screen", "", true, None).is_some());
    }

    #[test]
    fn kitty_cleanup_targets_owned_id_not_global_images() {
        assert_eq!(
            kitty_cleanup_command(12345, false),
            "\x1b_Ga=d,d=I,i=12345,q=2;\x1b\\",
        );
        let tmux = kitty_cleanup_command(12345, true);
        assert!(tmux.starts_with("\x1bPtmux;"));
        assert!(tmux.contains("i=12345"));
        assert!(!tmux.contains("d=A"));
    }

    #[test]
    fn sixel_requires_measured_cell_size_and_kitty_does_not() {
        assert!(TerminalImages::from_environment("", "foot", "", false, None).is_none());
        assert!(TerminalImages::from_environment("", "xterm-kitty", "", false, None).is_some());
        assert!(
            TerminalImages::from_environment("", "foot", "", false, Some(FontSize::new(8, 16)))
                .is_some()
        );
    }

    #[test]
    fn preview_dimensions_fit_both_axes_and_reflow_on_resize() {
        let cell = FontSize::new(8, 16);
        assert_eq!(preview_size(1024, 1024, 80, cell), Some(Size::new(40, 20)));
        assert_eq!(preview_size(1024, 1024, 18, cell), Some(Size::new(16, 8)));
        assert_eq!(preview_size(2, 2, 0, cell), None);
        assert_eq!(preview_size(1, 16000, 80, cell), Some(Size::new(1, 20)));
    }

    #[test]
    fn decoder_rejects_corruption_and_oversized_input_before_allocation() {
        let path = std::env::temp_dir().join(format!("roder-preview-{}.png", std::process::id()));
        std::fs::write(&path, b"not a PNG").unwrap();
        assert!(decode(&path).is_err());
        let file = File::create(&path).unwrap();
        file.set_len(MAX_INPUT_BYTES + 1).unwrap();
        assert_eq!(decode(&path).unwrap_err(), "image exceeds 32 MiB");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn actual_kitty_pixels_are_encoded_into_the_render_buffer() {
        use ratatui::{buffer::Buffer, layout::Rect};

        let path =
            std::env::temp_dir().join(format!("roder-preview-pixels-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(8, 16, image::Rgba([30, 40, 50, 255]))
            .save(&path)
            .unwrap();
        let terminal = TerminalImages {
            protocol: ProtocolType::Kitty,
            cell: FontSize::new(8, 16),
            tmux: false,
        };
        let loaded = load(&path, 20, terminal).unwrap();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 5));
        loaded.render_into(buffer.area, SignedPosition::from((2, 1)), &mut buffer);
        let encoded = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(encoded.contains("_Gq=2,"));
        assert!(encoded.contains("a=T,U=1,f=32"));
        assert_eq!(loaded.size, Size::new(1, 1));
        // A buffer-only test has not transmitted pixels to the real terminal.
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decoder_rejects_an_excessive_declared_raster() {
        use image::ImageEncoder;
        let path = std::env::temp_dir().join(format!(
            "roder-preview-dimensions-{}.png",
            std::process::id()
        ));
        let file = File::create(&path).unwrap();
        image::codecs::png::PngEncoder::new(file)
            .write_image(
                &vec![0; (MAX_DIMENSION as usize + 1) * 4],
                MAX_DIMENSION + 1,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assert_eq!(decode(&path).unwrap_err(), "invalid or oversized image");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn actual_sixel_pixels_are_encoded_into_the_render_buffer() {
        use ratatui::{buffer::Buffer, layout::Rect};

        let path =
            std::env::temp_dir().join(format!("roder-preview-sixel-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(8, 16, image::Rgba([30, 40, 50, 255]))
            .save(&path)
            .unwrap();
        let loaded = load(
            &path,
            20,
            TerminalImages {
                protocol: ProtocolType::Sixel,
                cell: FontSize::new(8, 16),
                tmux: false,
            },
        )
        .unwrap();
        assert!(matches!(loaded.protocol, EncodedImage::Sixel(_)));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 5));
        loaded.render_into(buffer.area, SignedPosition::from((2, 1)), &mut buffer);
        let encoded = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(encoded.contains("\x1bP"));
        assert!(encoded.contains('q'));
        std::fs::remove_file(path).unwrap();
    }
}
