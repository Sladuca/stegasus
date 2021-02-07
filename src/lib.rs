use wasm_bindgen::prelude::*;

use anyhow::{anyhow, Error};
use bytemuck::cast_slice;
use image::codecs::png::{PngDecoder, PngEncoder};
use image::{ColorType, DynamicImage};
use std::io::Cursor;

const ECC_BLOCK_LEN: usize = 255;
// use reed-solomon ECC with k = 32, max 16 bytes corrected
// may be overkill, can prolly reduce, which will lead to better throughput
const ECC_CODE_LEN: usize = 32;
const ECC_DATA_LEN: usize = ECC_BLOCK_LEN - 32;

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

#[wasm_bindgen]
pub fn encode_img(carrier: &[u8], width: u32, height: u32, input: &[u8]) -> Vec<u8> {
    encode_img_inner(carrier, width, height, input).unwrap()
}

pub fn encode_img_inner(
    carrier: &[u8],
    width: u32,
    height: u32,
    input: &[u8],
) -> Result<Vec<u8>, Error> {
    let decoder = PngDecoder::new(carrier).map_err(|e| anyhow!(e))?;
    let img = DynamicImage::from_decoder(decoder).map_err(|e| anyhow!(e))?;

    let blocks = input_ecc(input);

    // error if image is too small
    if width * height < (ECC_BLOCK_LEN * blocks.len()) as u32 {
        return Err(anyhow!("image too small!"));
    }

    let block_region_size = width * height / blocks.len() as u32;
    let block_region_offset = (block_region_size - (ECC_BLOCK_LEN * 8) as u32) / 2;

    let mut img = img.into_rgba16();

    for (i, block) in blocks.iter().enumerate() {
        let block_start = (i as u32) * block_region_size + block_region_offset;
        let mut chan = 0;
        for j in block_start..block_start + ((ECC_BLOCK_LEN * 8) as u32) {
            let x = j % width;
            let y = j / width;
            let pixel = img.get_pixel_mut(x as u32, y as u32);
            if j < ((ECC_DATA_LEN * 8) as u32) {
                pixel[chan] = (pixel[chan] & 0xFFFE)
                    | ((block.data()[(j / 8) as usize] >> (j % 8)) & (0x1)) as u16;
            } else {
                pixel[chan] = (pixel[chan] & 0xFFFE)
                    | ((block.ecc()[(j / 8) as usize] >> (j % 8)) & (0x1)) as u16;
            }
            chan = (chan + 1) % 3;
        }
    }

    let mut buf = Vec::new();
    let cursor = Cursor::new(&mut buf);

    let encoder = PngEncoder::new(cursor);
    encoder
        .encode(cast_slice(img.as_raw()), width, height, ColorType::Rgb16)
        .map_err(|e| anyhow!(e))?;

    Ok(buf)
}
