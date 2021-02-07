use anyhow::Error;
use image::codecs::png::{PngDecoder, PngEncoder};
use image::{DynamicImage, ImageFormat};
use rug::{float::Round, Float};
use std::io::Read;

const ECC_BLOCK_LEN: usize = 255;
// use reed-solomon ECC with k = 32, max 16 bytes corrected
// may be overkill, can prolly reduce, which will lead to better throughput
const ECC_CODE_LEN: usize = 32;
const ECC_DATA_LEN: usize = ECC_BLOCK_LEN - 32;

/// enum over the carriers supported by stegasus
pub enum StegCarrier<R: Read> {
    Image(R, ImageFormat),
}

/// encodes input data using reed-solomon block ECC
/// returns a vector of encoded blocks
fn input_ecc(input: &[u8]) -> Vec<reed_solomon::Buffer> {
    let num_blocks = (input.len() + ECC_DATA_LEN - 1) / ECC_DATA_LEN;
    let encoder = reed_solomon::Encoder::new(ECC_CODE_LEN);

    let mut blocks: Vec<reed_solomon::Buffer> = Vec::with_capacity(num_blocks);
    for i in 0..num_blocks {
        let encoded = encoder.encode(input);
        blocks[i] = encoded;
    }
    blocks
}

pub fn lift_to_spiral_coords(width: u32, height: u32, i: usize) -> Option<(u32, u32)> {
    if (i as u32) >= width * height {
        return None;
    }

    let b = 2 * height + 2 * width - 4;
    let mut radical = Float::with_val(53, b * b - 16 * (i as u32) + 64);
    radical.sqrt_round(Round::Down);
    let num = Float::with_val(53, b) - radical;

    let level: Float = num / 8.0;

    let level = match level.trunc().to_u32_saturating() {
        Some(level) => level,
        None => return None,
    };

    let mut coords = (level, level);
    let start_coords_index = if level == 0 {
        0
    } else {
        (2 * height + 2 * width) * level - 4 * level * level - 4 * level + 4
    };
    println!("level: {}, start_index: {}", level, start_coords_index);
    let mut rem = (i as u32) - start_coords_index;
    println!("rem: {}", rem);
    if rem <= height - 2 * level - 1 {
        return Some((coords.0, coords.1 + rem));
    }
    coords = (coords.0, coords.1 + (height - 2 * level - 1));
    rem -= height - 2 * level - 1;
    println!("rem: {}", rem);
    if rem <= width - 2 * level - 1 {
        return Some((coords.0 + rem, coords.1));
    }
    coords = (coords.0 + (width - 2 * level - 1), coords.1);
    rem -= width - 2 * level - 1;
    println!("rem: {}", rem);
    if rem <= height - 2 * level - 1 {
        return Some((coords.0, coords.1 - rem));
    }
    coords = (coords.0, coords.1 - (height - 2 * level - 1));
    rem -= height - 2 * level - 1;
    println!("rem: {}", rem);
    Some((coords.0 - rem, coords.1))
}

fn img_iter_spiral(width: u32, height: u32, num_steps: usize) -> Vec<(u32, u32)> {
    let max_index = (width * height) as usize;
    let mut coords = Vec::with_capacity(num_steps);

    let step_size = max_index / num_steps;
    let mut i = 0;
    for step in 0..num_steps {
        coords[step] = lift_to_spiral_coords(width, height, i).unwrap();
        i += step_size;
    }

    coords
}

/*
impl<R: Read> StegCarrier<R> {
    fn encode<O: Read>(self, input: &[u8]) -> Result<StegCarrier<O>, Error> {
        match self {
            Self::Image(rdr, ImageFormat::Png) => {
                let png_decoder = PngDecoder::new(rdr).map_err(|e| anyhow!(e))?;
                let mut img = DynamicImage::from_decoder(pnc_decoder).map_err(|e| anyhow!(e))?;
                let (width, height) = img.dimensions();

                let blocks = input_ecc(input);

                // error if image is too small
                if width * height < ECC_BLOCK_LEN * blocks.len() {
                    return anyhow!("image too small!");
                }




            }
            _ => anyhow!("unsupported carrier type!")
        }
    }
}
*/

#[cfg(test)]
mod tests {
    use crate::lift_to_spiral_coords;

    enum SpiralState {
        Down,
        Right,
        Up,
        Left,
    }

    fn step_spiral(h: u32, w: u32, i: u32, mut state: SpiralState, level: u32, curr: (u32, u32)) {
        let coords = match lift_to_spiral_coords(h, w, i as usize) {
            Some(coords) => coords,
            None => return,
        };
        assert_eq!(coords, curr);
        println!("{} -> ({}, {})", i, coords.0, coords.1);
        match state {
            SpiralState::Down => {
                if curr.1 == h - level - 1 {
                    state = SpiralState::Right;
                    step_spiral(h, w, i + 1, state, level, (curr.0 + 1, curr.1));
                } else {
                    step_spiral(h, w, i + 1, state, level, (curr.0, curr.1 + 1));
                }
            }
            SpiralState::Right => {
                if curr.0 == w - level - 1 {
                    state = SpiralState::Up;
                    step_spiral(h, w, i + 1, state, level, (curr.0, curr.1 - 1));
                } else {
                    step_spiral(h, w, i + 1, state, level, (curr.0 + 1, curr.1));
                }
            }
            SpiralState::Up => {
                if curr.1 == level {
                    state = SpiralState::Left;
                    step_spiral(h, w, i + 1, state, level + 1, (curr.0 - 1, curr.1));
                } else {
                    step_spiral(h, w, i + 1, state, level, (curr.0, curr.1 - 1));
                }
            }
            SpiralState::Left => {
                if curr.0 == level {
                    state = SpiralState::Down;
                    step_spiral(h, w, i + 1, state, level, (curr.0, curr.1 + 1));
                } else {
                    step_spiral(h, w, i + 1, state, level, (curr.0 - 1, curr.1));
                }
            }
        }
    }

    fn test_with_size(height: u32, width: u32) {
        step_spiral(height, width, 0, SpiralState::Down, 0, (0, 0));
    }

    #[test]
    fn test_spiral_lift() {
        test_with_size(128, 128);
        test_with_size(128, 256);
        test_with_size(256, 128);
        test_with_size(128, 1024);
        test_with_size(1024, 128);
        test_with_size(100, 99);
        test_with_size(101, 97);
        test_with_size(113, 39);
    }
}
