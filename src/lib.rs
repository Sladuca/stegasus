use wasm_bindgen::prelude::*;

use anyhow::{anyhow, Error};
use bytemuck::cast_slice;
use byteorder::{ByteOrder, LittleEndian};
use image::codecs::png::{PngDecoder, PngEncoder};
use image::{ColorType, DynamicImage};
use std::io::Cursor;

const ECC_BLOCK_LEN: usize = 255;
// use reed-solomon ECC with k = 32, max 16 bytes corrected
// may be overkill, can prolly reduce, which will lead to better throughput
const ECC_CODE_LEN: usize = 32;
const ECC_DATA_LEN: usize = ECC_BLOCK_LEN - 32;

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

    let blocks = encode_ecc(input);

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

pub fn decode_img_inner(img: &[u8], width: u32, height: u32) -> Result<Vec<u8>, Error> {
    let decoder = PngDecoder::new(img).map_err(|e| anyhow!(e))?;
    let img = DynamicImage::from_decoder(decoder).map_err(|e| anyhow!(e))?;
    let mut img = img.into_rgba16();

    // get len block
    let mut chan = 0;
    let mut block: [u8; ECC_BLOCK_LEN] = [0; ECC_BLOCK_LEN];
    for (i, pixel) in img.pixels().take(ECC_BLOCK_LEN * 8).enumerate() {
        let bit = pixel[chan] & 0x01;
        block[i / 8] |= (bit << (i % 8)) as u8;
        chan = (chan + 1) % 3;
    }
    let decoded_len_block = decode_ecc(vec![block])?;

    let data_len_bytes = &decoded_len_block[0..std::mem::size_of::<usize>()];
    let data_len = LittleEndian::read_uint(data_len_bytes, std::mem::size_of::<usize>()) as usize;

    let num_blocks = (data_len + ECC_DATA_LEN - 1) / ECC_DATA_LEN;

    // block regions determined by num_blocks + 1, not num_blocks bc data len block
    let block_region_size = width * height / (num_blocks + 1) as u32;
    let block_region_offset = (block_region_size - (ECC_BLOCK_LEN * 8) as u32) / 2;

    // get blocks
    let mut blocks = Vec::with_capacity(num_blocks);
    for i in 1..num_blocks + 1 {
        let block_start = (i as u32) * block_region_size + block_region_offset;
        let mut block: [u8; ECC_BLOCK_LEN] = [0; ECC_BLOCK_LEN];
        let mut chan = 0;
        for j in block_start..block_start + ((ECC_BLOCK_LEN * 8) as u32) {
            let x = j % width;
            let y = j / width;
            let pixel = img.get_pixel_mut(x as u32, y as u32);
            let bit = (pixel[chan] & 0xFFFE) as u8;
            block[j as usize / 8] |= bit << (j % 8);
            chan = (chan + 1) % 3;
        }
        blocks.push(block);
    }

    let decoded = decode_ecc(blocks)?;
    Ok(decoded)
}

/// encodes input data using reed-solomon block ECC
/// returns a vector of encoded blocks with an extra block at the beginning saying how long the
/// data is
fn encode_ecc(input: &[u8]) -> Vec<reed_solomon::Buffer> {
    let num_blocks = (input.len() + ECC_DATA_LEN - 1) / ECC_DATA_LEN;
    let encoder = reed_solomon::Encoder::new(ECC_CODE_LEN);

    let mut blocks: Vec<reed_solomon::Buffer> = Vec::with_capacity(num_blocks + 1);
    blocks.push(encoder.encode(&input.len().to_le_bytes()));
    for i in 0..num_blocks {
        let offset = i * ECC_DATA_LEN;
        if input.len() - offset < ECC_DATA_LEN {
            blocks.push(encoder.encode(&input[offset..]));
        } else {
            blocks.push(encoder.encode(&input[offset..offset + ECC_DATA_LEN]));
        }
    }
    blocks
}

fn decode_ecc(blocks: Vec<[u8; ECC_BLOCK_LEN]>) -> Result<Vec<u8>, Error> {
    let decoder = reed_solomon::Decoder::new(ECC_CODE_LEN);
    let mut buf = Vec::new();
    for block in blocks.iter() {
        let decoded = decoder
            .correct(&block[..], None)
            .map_err(|_| anyhow!("message data is corrupted!"))?;
        buf.extend_from_slice(decoded.data());
    }
    Ok(buf)
}
