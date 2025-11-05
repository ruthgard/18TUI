use std::collections::HashMap;

use once_cell::sync::Lazy;

const FONT_HEIGHT: usize = 7;
const FONT_WIDTH: usize = 5;
const SHADOW_OFFSET: usize = 2;
const FILL_CHAR: char = '█';
const OUTLINE_CHAR: char = '░';

type Glyph = [&'static str; FONT_HEIGHT];

static GLYPHS: Lazy<HashMap<char, Glyph>> = Lazy::new(|| {
    HashMap::from([
        (
            'A',
            [
                " 111 ", "1   1", "1   1", "11111", "1   1", "1   1", "1   1",
            ],
        ),
        (
            'B',
            [
                "1111 ", "1   1", "1   1", "1111 ", "1   1", "1   1", "1111 ",
            ],
        ),
        (
            'C',
            [
                " 111 ", "1   1", "1    ", "1    ", "1    ", "1   1", " 111 ",
            ],
        ),
        (
            'D',
            [
                "1111 ", "1   1", "1   1", "1   1", "1   1", "1   1", "1111 ",
            ],
        ),
        (
            'E',
            [
                "11111", "1    ", "1    ", "1111 ", "1    ", "1    ", "11111",
            ],
        ),
        (
            'F',
            [
                "11111", "1    ", "1    ", "1111 ", "1    ", "1    ", "1    ",
            ],
        ),
        (
            'G',
            [
                " 111 ", "1   1", "1    ", "1 111", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            'H',
            [
                "1   1", "1   1", "1   1", "11111", "1   1", "1   1", "1   1",
            ],
        ),
        (
            'I',
            [
                "11111", "  1  ", "  1  ", "  1  ", "  1  ", "  1  ", "11111",
            ],
        ),
        (
            'J',
            [
                "    1", "    1", "    1", "    1", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            'K',
            [
                "1   1", "1  1 ", "1 1  ", "11   ", "1 1  ", "1  1 ", "1   1",
            ],
        ),
        (
            'L',
            [
                "1    ", "1    ", "1    ", "1    ", "1    ", "1    ", "11111",
            ],
        ),
        (
            'M',
            [
                "1   1", "11  1", "1 1 1", "1 1 1", "1   1", "1   1", "1   1",
            ],
        ),
        (
            'N',
            [
                "1   1", "11  1", "1 1 1", "1  11", "1   1", "1   1", "1   1",
            ],
        ),
        (
            'O',
            [
                " 111 ", "1   1", "1   1", "1   1", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            'P',
            [
                "1111 ", "1   1", "1   1", "1111 ", "1    ", "1    ", "1    ",
            ],
        ),
        (
            'Q',
            [
                " 111 ", "1   1", "1   1", "1   1", "1 1 1", "1  1 ", " 11 1",
            ],
        ),
        (
            'R',
            [
                "1111 ", "1   1", "1   1", "1111 ", "1  1 ", "1   1", "1   1",
            ],
        ),
        (
            'S',
            [
                " 111 ", "1   1", "1    ", " 111 ", "    1", "1   1", " 111 ",
            ],
        ),
        (
            'T',
            [
                "11111", "  1  ", "  1  ", "  1  ", "  1  ", "  1  ", "  1  ",
            ],
        ),
        (
            'U',
            [
                "1   1", "1   1", "1   1", "1   1", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            'V',
            [
                "1   1", "1   1", "1   1", "1   1", "1   1", " 1 1 ", "  1  ",
            ],
        ),
        (
            'W',
            [
                "1   1", "1   1", "1   1", "1 1 1", "1 1 1", "1 1 1", " 1 1 ",
            ],
        ),
        (
            'X',
            [
                "1   1", "1   1", " 1 1 ", "  1  ", " 1 1 ", "1   1", "1   1",
            ],
        ),
        (
            'Y',
            [
                "1   1", "1   1", " 1 1 ", "  1  ", "  1  ", "  1  ", "  1  ",
            ],
        ),
        (
            'Z',
            [
                "11111", "    1", "   1 ", "  1  ", " 1   ", "1    ", "11111",
            ],
        ),
        (
            '0',
            [
                " 111 ", "1   1", "1  11", "1 1 1", "11  1", "1   1", " 111 ",
            ],
        ),
        (
            '1',
            [
                "  1  ", " 11  ", "1 1  ", "  1  ", "  1  ", "  1  ", "11111",
            ],
        ),
        (
            '2',
            [
                " 111 ", "1   1", "    1", "   1 ", "  1  ", " 1   ", "11111",
            ],
        ),
        (
            '3',
            [
                " 111 ", "1   1", "    1", "  11 ", "    1", "1   1", " 111 ",
            ],
        ),
        (
            '4',
            [
                "   1 ", "  11 ", " 1 1 ", "1  1 ", "11111", "   1 ", "   1 ",
            ],
        ),
        (
            '5',
            [
                "11111", "1    ", "1    ", "1111 ", "    1", "1   1", " 111 ",
            ],
        ),
        (
            '6',
            [
                " 111 ", "1   1", "1    ", "1111 ", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            '7',
            [
                "11111", "    1", "   1 ", "  1  ", " 1   ", "1    ", "1    ",
            ],
        ),
        (
            '8',
            [
                " 111 ", "1   1", "1   1", " 111 ", "1   1", "1   1", " 111 ",
            ],
        ),
        (
            '9',
            [
                " 111 ", "1   1", "1   1", " 1111", "    1", "1   1", " 111 ",
            ],
        ),
        (
            '-',
            [
                "     ", "     ", "     ", "11111", "     ", "     ", "     ",
            ],
        ),
        (
            '/',
            [
                "    1", "   1 ", "   1 ", "  1  ", " 1   ", "1    ", "1    ",
            ],
        ),
        (
            ':',
            [
                "     ", "  1  ", "     ", "     ", "  1  ", "     ", "     ",
            ],
        ),
        (
            ' ',
            [
                "     ", "     ", "     ", "     ", "     ", "     ", "     ",
            ],
        ),
        (
            '?',
            [
                " 111 ", "1   1", "    1", "   1 ", "  1  ", "     ", "  1  ",
            ],
        ),
    ])
});

/// Render the provided text using the block font with layered outline.
pub fn render(text: &str) -> Vec<String> {
    let content: Vec<char> = text.chars().map(|c| c.to_ascii_uppercase()).collect();
    if content.is_empty() {
        return vec![String::new(); FONT_HEIGHT + SHADOW_OFFSET];
    }

    let canvas_height = FONT_HEIGHT + SHADOW_OFFSET;
    let glyph_width = FONT_WIDTH * 2; // double width for chunky appearance
    let spacing = 2;
    let total_width =
        content.len() * glyph_width + (content.len().saturating_sub(1)) * spacing + SHADOW_OFFSET;
    let mut canvas = vec![vec![' '; total_width]; canvas_height];

    for (index, ch) in content.iter().enumerate() {
        let glyph = GLYPHS.get(ch).or_else(|| GLYPHS.get(&'?')).unwrap();
        let x_offset = index * (glyph_width + spacing);
        paint_glyph(&mut canvas, glyph, x_offset);
    }

    canvas
        .into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

fn paint_glyph(canvas: &mut [Vec<char>], glyph: &Glyph, x_offset: usize) {
    for (row_idx, row) in glyph.iter().enumerate() {
        for (col_idx, symbol) in row.chars().enumerate() {
            if symbol != '1' {
                continue;
            }
            let base_y = row_idx;
            let base_x = x_offset + col_idx * 2;
            apply_fill(canvas, base_y, base_x);
        }
    }
}

fn apply_fill(canvas: &mut [Vec<char>], y: usize, x: usize) {
    place(
        canvas,
        y + SHADOW_OFFSET,
        x + SHADOW_OFFSET * 2,
        OUTLINE_CHAR,
    );
    place(
        canvas,
        y + SHADOW_OFFSET,
        x + SHADOW_OFFSET * 2 + 1,
        OUTLINE_CHAR,
    );
    place(
        canvas,
        y + SHADOW_OFFSET - 1,
        x + SHADOW_OFFSET * 2,
        OUTLINE_CHAR,
    );
    place(
        canvas,
        y + SHADOW_OFFSET - 1,
        x + SHADOW_OFFSET * 2 + 1,
        OUTLINE_CHAR,
    );

    place(canvas, y, x, FILL_CHAR);
    place(canvas, y, x + 1, FILL_CHAR);
}

fn place(canvas: &mut [Vec<char>], y: usize, x: usize, ch: char) {
    if y >= canvas.len() || x >= canvas[y].len() {
        return;
    }
    let cell = &mut canvas[y][x];
    if *cell == ' ' || (*cell == OUTLINE_CHAR && ch == FILL_CHAR) {
        *cell = ch;
    }
}
