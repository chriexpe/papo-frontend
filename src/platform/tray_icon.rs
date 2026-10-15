//! Pixels shared by the Linux and Windows tray icons.

pub fn render(size: u32, pending: bool) -> Option<image::RgbaImage> {
    let source = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?;
    let mut icon = source
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    if pending {
        let center = size as f32 * 0.78;
        let radius = size as f32 * 0.17;
        for (x, y, pixel) in icon.enumerate_pixels_mut() {
            let distance = ((x as f32 + 0.5 - center).powi(2)
                + (y as f32 + 0.5 - (size as f32 - center)).powi(2))
            .sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                use image::Pixel;
                pixel.blend(&image::Rgba([
                    229,
                    57,
                    53,
                    (coverage * 255.0).round() as u8,
                ]));
            }
        }
    }
    Some(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_is_red_and_only_changes_the_top_right_corner() {
        for size in [22, 32, 64] {
            let plain = render(size, false).unwrap();
            let marked = render(size, true).unwrap();
            assert_eq!(plain.dimensions(), marked.dimensions());
            assert_eq!(
                *marked.get_pixel(size * 78 / 100, size * 22 / 100),
                image::Rgba([229, 57, 53, 255])
            );
            for (x, y, pixel) in plain.enumerate_pixels() {
                if x < size / 2 || y >= size / 2 {
                    assert_eq!(pixel, marked.get_pixel(x, y));
                }
            }
            assert_eq!(plain, render(size, false).unwrap());
        }
    }
}
