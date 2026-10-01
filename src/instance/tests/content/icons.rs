// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn half_block_pixels_keep_the_top_and_bottom_colors() {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(1, 2, |_, y| {
        if y == 0 {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    }));
    let rows = make_icon_pixels_from_image(&image, 1, 1);
    let cell = &rows[0][0];
    assert_eq!(cell.symbol, '▀');
    assert_eq!((cell.fg_r, cell.fg_g, cell.fg_b), (255, 0, 0));
    assert_eq!((cell.bg_r, cell.bg_g, cell.bg_b), (0, 0, 255));
}

#[test]
fn quadrant_raster_has_requested_dimensions() {
    let image =
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([12, 34, 56])));
    let rows = make_icon_quadrants_from_image(&image, 7, 3);

    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.len() == 7));
    assert!(rows.iter().flatten().all(|cell| cell.symbol == '\u{2588}'));
}

#[test]
fn fallback_icon_is_square_and_contains_a_separate_question_mark_dot() {
    let icon = fallback_icon();

    assert_eq!(icon.len(), 3);
    assert!(icon.iter().all(|row| row.len() == 6));
    assert_eq!(icon[2][2].bg_r, 45);
    assert_eq!(icon[2][2].fg_r, 150);
    assert_eq!(icon[2][3].fg_r, 150);
}
