//! One-bit images in printer orientation.

/// A black-and-white image, one byte per pixel (1 = black). `width` runs across the print head, `height`
/// along the feed; row 0 leaves the printer first.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}x{}, {} black)", self.width, self.height, self.black_pixels())
    }
}

impl Bitmap {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; (width * height) as usize] }
    }

    /// Threshold an 8-bit coverage buffer (255 = full ink) into black and white.
    pub fn from_coverage(width: u32, height: u32, coverage: &[u8], threshold: u8) -> Self {
        assert_eq!(coverage.len(), (width * height) as usize);
        Self { width, height, pixels: coverage.iter().map(|&c| (c >= threshold) as u8).collect() }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.pixels[(y * self.width + x) as usize] != 0
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, black: bool) {
        self.pixels[(y * self.width + x) as usize] = black as u8;
    }

    pub fn row(&self, y: u32) -> &[u8] {
        let w = self.width as usize;
        &self.pixels[y as usize * w..(y as usize + 1) * w]
    }

    pub fn black_pixels(&self) -> usize {
        self.pixels.iter().filter(|&&p| p != 0).count()
    }

    pub fn invert(&mut self) {
        for p in &mut self.pixels {
            *p ^= 1;
        }
    }

    /// Rotate 90° clockwise: the left edge becomes the top edge.
    pub fn rotate_cw(&self) -> Self {
        let mut out = Bitmap::new(self.height, self.width);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    out.set(self.height - 1 - y, x, true);
                }
            }
        }
        out
    }

    /// Bounding box of the black pixels as (x0, y0, x1, y1), exclusive end.
    pub fn ink_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x0 != u32::MAX).then_some((x0, y0, x1, y1))
    }

    /// Grayscale PNG (black ink on white) for previews and tests.
    pub fn to_png(&self) -> Vec<u8> {
        let gray: Vec<u8> = self.pixels.iter().map(|&p| if p != 0 { 0 } else { 255 }).collect();
        encode_png_gray(self.width, self.height, &gray)
    }
}

pub fn encode_png_gray(width: u32, height: u32, gray: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(gray).expect("png data");
    }
    out
}

pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(rgba).expect("png data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_cw_moves_left_edge_to_top() {
        let mut b = Bitmap::new(3, 2);
        b.set(0, 0, true); // top-left
        let r = b.rotate_cw();
        assert_eq!((r.width, r.height), (2, 3));
        assert!(r.get(1, 0), "top-left ends up top-right");
        assert_eq!(r.black_pixels(), 1);
    }
}
