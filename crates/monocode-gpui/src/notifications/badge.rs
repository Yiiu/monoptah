//! The Windows taskbar overlay's portable 16×16 pixel art.

pub(crate) const BADGE_SIDE: usize = 16;
pub(crate) type BadgePixels = [[u8; 4]; BADGE_SIDE * BADGE_SIDE];

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
const PLUS: [u8; 5] = [0, 0b010, 0b111, 0b010, 0];

pub(crate) fn badge_pixels(count: usize, fill: [u8; 4], ink: [u8; 4]) -> BadgePixels {
    let mut pixels = [[0; 4]; BADGE_SIDE * BADGE_SIDE];
    for (index, pixel) in pixels.iter_mut().enumerate() {
        // Double the pixel-center offsets about the center of the icon.
        let x = (2 * (index % BADGE_SIDE)) as i32 - 15;
        let y = (2 * (index / BADGE_SIDE)) as i32 - 15;
        if x * x + y * y <= 16 * 16 {
            *pixel = fill;
        }
    }
    if count > 9 {
        draw_glyph(&mut pixels, 4, DIGITS[9], ink);
        draw_glyph(&mut pixels, 8, PLUS, ink);
    } else {
        draw_glyph(&mut pixels, 6, DIGITS[count], ink);
    }
    pixels
}

fn draw_glyph(pixels: &mut BadgePixels, x: usize, glyph: [u8; 5], ink: [u8; 4]) {
    for (row, bits) in glyph.into_iter().enumerate() {
        for column in 0..3 {
            if bits & (0b100 >> column) != 0 {
                pixels[(row + 5) * BADGE_SIDE + x + column] = ink;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_draws_a_transparent_circle_with_a_centered_digit() {
        let fill = [251, 191, 36, 255];
        let ink = [0, 0, 0, 255];
        let pixels = badge_pixels(1, fill, ink);
        assert_eq!(pixels[0], [0; 4]);
        assert_eq!(pixels[8], fill);
        assert_eq!(pixels[5 * BADGE_SIDE + 7], ink);
        assert_eq!(pixels[5 * BADGE_SIDE + 6], fill);
        assert_eq!(pixels[9 * BADGE_SIDE + 8], ink);
    }

    #[test]
    fn overlay_caps_double_digit_counts_at_nine_plus() {
        let fill = [251, 191, 36, 255];
        let ink = [0, 0, 0, 255];
        let nine = badge_pixels(9, fill, ink);
        let nine_plus = badge_pixels(10, fill, ink);
        assert_ne!(nine, nine_plus);
        assert_eq!(nine_plus, badge_pixels(100, fill, ink));
        assert_eq!(nine_plus[5 * BADGE_SIDE + 4..5 * BADGE_SIDE + 7], [ink; 3]);
        assert_eq!(nine_plus[7 * BADGE_SIDE + 8..7 * BADGE_SIDE + 11], [ink; 3]);
        assert_eq!(nine_plus[6 * BADGE_SIDE + 8], fill);
        assert_eq!(nine_plus[6 * BADGE_SIDE + 9], ink);
    }
}
