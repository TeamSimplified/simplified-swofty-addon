use std::io::BufReader;
use std::path::Path;

pub use swofty_formats::texture::Image;

pub fn load_png(path: &Path) -> Result<Image, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| fail(&e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| fail(&e))?;
    let size = reader.output_buffer_size().ok_or_else(|| fail(&"too large"))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| fail(&e))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(fail(&"indexed colour didn't expand")),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn load_jpeg(path: &Path) -> Result<Image, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| fail(&e))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    let pixels = decoder.decode().map_err(|e| fail(&e))?;
    let info = decoder.info().ok_or_else(|| fail(&"no picture"))?;
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        jpeg_decoder::PixelFormat::L16 => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], 255])
            .collect(),
        jpeg_decoder::PixelFormat::CMYK32 => pixels
            .chunks_exact(4)
            .flat_map(|p| {
                let k = 255 - u32::from(p[3]);
                let c = |v: u8| ((255 - u32::from(v)) * k / 255) as u8;
                [c(p[0]), c(p[1]), c(p[2]), 255]
            })
            .collect(),
    };
    Ok(Image {
        width: u32::from(info.width),
        height: u32::from(info.height),
        rgba,
    })
}

/// An SVG drawn with its longest side at `side` pixels.
fn load_svg(path: &Path, side: u32) -> Result<Image, String> {
    use resvg::{tiny_skia, usvg};
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let data = std::fs::read(path).map_err(|e| fail(&e))?;
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_data(&data, &options).map_err(|e| fail(&e))?;
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height()).max(1.0);
    let (w, h) = (
        ((size.width() * scale).round() as u32).max(1),
        ((size.height() * scale).round() as u32).max(1),
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| fail(&"too large"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps colour premultiplied by alpha.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(Image {
        width: w,
        height: h,
        rgba,
    })
}

/// The pictures an import takes: PNG, JPEG or SVG.
pub const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "svg"];

/// Any picture an import takes, no larger than `side` along its longest side.
pub fn load_picture(path: &Path, side: u32) -> Result<Image, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let picture = match ext.as_str() {
        "svg" => return load_svg(path, side),
        "jpg" | "jpeg" => load_jpeg(path)?,
        _ => load_png(path)?,
    };
    let longest = picture.width.max(picture.height);
    Ok(if longest > side {
        let (w, h) = (picture.width * side / longest, picture.height * side / longest);
        fit(&picture, w.max(1), h.max(1))
    } else {
        picture
    })
}

/// The picture centred in a `width` by `height` slot, filling `percent` of it
/// without stretching, on a transparent background or on `background`.
pub fn place(src: &Image, width: u32, height: u32, percent: u32, background: Option<[u8; 3]>) -> Image {
    let pct = percent.clamp(20, 100);
    let (iw, ih) = ((width * pct / 100).max(1), (height * pct / 100).max(1));
    let inner = fit(src, iw, ih);
    let (ox, oy) = ((width - iw) / 2, (height - ih) / 2);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..ih {
        let from = (y * iw * 4) as usize;
        let to = (((oy + y) * width + ox) * 4) as usize;
        rgba[to..to + (iw * 4) as usize].copy_from_slice(&inner.rgba[from..from + (iw * 4) as usize]);
    }
    if let Some(bg) = background {
        for p in rgba.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            for k in 0..3 {
                p[k] = ((u32::from(p[k]) * a + u32::from(bg[k]) * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    Image { width, height, rgba }
}

pub fn save_png(image: &Image, path: &Path) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let tmp = path.with_extension("png.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| fail(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| fail(&e))?;
    writer.write_image_data(&image.rgba).map_err(|e| fail(&e))?;
    writer.finish().map_err(|e| fail(&e))?;
    std::fs::rename(&tmp, path).map_err(|e| fail(&e))
}

// Area average of the source box under each output pixel, weighting colour by
// alpha so transparent pixels don't darken the edges.
fn resample(src: &Image, x0: f32, y0: f32, sw: f32, sh: f32, width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for oy in 0..height {
        let fy0 = y0 + sh * oy as f32 / height as f32;
        let fy1 = y0 + sh * (oy + 1) as f32 / height as f32;
        let ys = (fy0.floor() as u32).min(src.height - 1);
        let ye = (fy1.ceil() as u32).clamp(ys + 1, src.height);
        for ox in 0..width {
            let fx0 = x0 + sw * ox as f32 / width as f32;
            let fx1 = x0 + sw * (ox + 1) as f32 / width as f32;
            let xs = (fx0.floor() as u32).min(src.width - 1);
            let xe = (fx1.ceil() as u32).clamp(xs + 1, src.width);
            let (mut acc, mut alpha, mut n) = ([0f32; 3], 0f32, 0f32);
            for sy in ys..ye {
                for sx in xs..xe {
                    let p = &src.rgba[((sy * src.width + sx) * 4) as usize..][..4];
                    let a = f32::from(p[3]);
                    for k in 0..3 {
                        acc[k] += f32::from(p[k]) * a;
                    }
                    alpha += a;
                    n += 1.0;
                }
            }
            let c = |k: usize| {
                if alpha > 0.0 {
                    (acc[k] / alpha).round() as u8
                } else {
                    0
                }
            };
            out.extend([c(0), c(1), c(2), (alpha / n.max(1.0)).round() as u8]);
        }
    }
    out
}

/// The picture scaled to fit inside `width` by `height` without stretching, centred
/// on a transparent box of that size, which is how the game's own logos sit in
/// their slots.
pub fn fit(src: &Image, width: u32, height: u32) -> Image {
    let scale = (width as f32 / src.width as f32).min(height as f32 / src.height as f32);
    let w = ((src.width as f32 * scale).round() as u32).clamp(1, width);
    let h = ((src.height as f32 * scale).round() as u32).clamp(1, height);
    let inner = resample(src, 0.0, 0.0, src.width as f32, src.height as f32, w, h);
    let (ox, oy) = ((width - w) / 2, (height - h) / 2);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..h {
        let from = (y * w * 4) as usize;
        let to = (((oy + y) * width + ox) * 4) as usize;
        rgba[to..to + (w * 4) as usize].copy_from_slice(&inner[from..from + (w * 4) as usize]);
    }
    Image { width, height, rgba }
}

pub fn half(src: &Image) -> Image {
    let (w, h) = ((src.width / 2).max(1), (src.height / 2).max(1));
    Image {
        width: w,
        height: h,
        rgba: resample(src, 0.0, 0.0, src.width as f32, src.height as f32, w, h),
    }
}

/// A round colour dot with a soft edge and a faint rim, so white and black show
/// on either theme.
pub fn dot(hex: &str, size: u32) -> Option<Image> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let (r, g, b) = ((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let c = size as f32 / 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let a = (c - d).clamp(0.0, 1.0);
            let rim = (d - (c - 1.5)).clamp(0.0, 1.0) * 0.35;
            let mix = |ch: u8| (f32::from(ch) * (1.0 - rim) + 128.0 * rim).round() as u8;
            rgba.extend([mix(r), mix(g), mix(b), (a * 255.0).round() as u8]);
        }
    }
    Some(Image {
        width: size,
        height: size,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placed_picture_fills_its_share_of_the_slot() {
        let src = Image {
            width: 2,
            height: 2,
            rgba: [255u8, 0, 0, 255].repeat(4),
        };
        let out = place(&src, 10, 10, 50, None);
        let alpha = |x: u32, y: u32| out.rgba[((y * 10 + x) * 4 + 3) as usize];
        assert_eq!((alpha(0, 0), alpha(5, 5), alpha(9, 9)), (0, 255, 0));
        let solid = place(&src, 10, 10, 50, Some([0, 0, 0]));
        assert!(solid.rgba.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn a_wide_picture_is_padded_not_stretched() {
        let src = Image {
            width: 4,
            height: 2,
            rgba: [255u8, 0, 0, 255].repeat(8),
        };
        let out = fit(&src, 4, 4);
        let at = |x: u32, y: u32| out.rgba[((y * 4 + x) * 4 + 3) as usize];
        assert_eq!((at(0, 0), at(0, 1), at(0, 2), at(0, 3)), (0, 255, 255, 0));
    }
}
