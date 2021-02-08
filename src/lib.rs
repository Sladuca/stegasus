use wasm_bindgen::prelude::*;

use anyhow::{anyhow, Error};
use byteorder::{ByteOrder, LittleEndian};
use image::io::Reader;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

const ECC_BLOCK_LEN: usize = 255;
// use reed-solomon ECC with k = 32, max 16 bytes corrected
// may be overkill, can prolly reduce, which will lead to better throughput
const ECC_CODE_LEN: usize = 32;
const ECC_DATA_LEN: usize = ECC_BLOCK_LEN - 32;
const USIZE_SIZE: usize = 8;

#[wasm_bindgen]
pub fn encode_img(carrier: &[u8], input: &[u8]) -> Vec<u8> {
    console_error_panic_hook::set_once();
    encode_img_inner(carrier, input).unwrap()
}

#[wasm_bindgen]
pub fn decode_img(img: &[u8]) -> Vec<u8> {
    console_error_panic_hook::set_once();
    decode_img_inner(img).unwrap()
}

pub fn encode_img_inner(carrier: &[u8], input: &[u8]) -> Result<Vec<u8>, Error> {
    let img = Reader::with_format(Cursor::new(carrier), ImageFormat::Png)
        .decode()
        .map_err(|e| anyhow!(e))?;
    let mut img = img.into_rgba8();
    let (width, height) = img.dimensions();

    let blocks = encode_ecc(input);

    // error if image is too small
    if width * height < (ECC_BLOCK_LEN * blocks.len()) as u32 {
        return Err(anyhow!("image too small!"));
    }

    let block_region_size = (width * height) / blocks.len() as u32;

    for (i, block) in blocks.iter().enumerate() {
        let block_start = (i as u32) * block_region_size;
        let mut chan = 0;
        for j in 0..block.data().len() * 8 {
            let x = ((j as u32) + block_start) % width;
            let y = ((j as u32) + block_start) / width;
            let pixel = img.get_pixel_mut(x as u32, y as u32);
            let block_byte = block.data()[j / 8];
            let bit = (block_byte >> (j % 8)) & 0x1;
            pixel[chan] = pixel[chan] & 0xFE | bit;
            chan = (chan + 1) % 3;
        }
        for j in 0..block.ecc().len() * 8 {
            let x = ((j as u32) + block_start + ((block.data().len() * 8) as u32)) % width;
            let y = ((j as u32) + block_start + ((block.data().len() * 8) as u32)) / width;
            let pixel = img.get_pixel_mut(x as u32, y as u32);
            let block_byte = block.ecc()[j / 8];
            let bit = (block_byte >> (j % 8)) & 0x1;
            pixel[chan] = pixel[chan] & 0xFE | bit;
            chan = (chan + 1) % 3;
        }
    }

    let img = DynamicImage::ImageRgba8(img);

    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);

    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| anyhow!(e))?;
    Ok(buf)
}

pub fn decode_img_inner(img: &[u8]) -> Result<Vec<u8>, Error> {
    let img = Reader::with_format(Cursor::new(img), ImageFormat::Png)
        .decode()
        .map_err(|e| anyhow!(e))?;
    let mut img = img.into_rgba8();
    let (width, height) = img.dimensions();

    // get len block
    let mut chan = 0;
    let mut block = vec![0; USIZE_SIZE + ECC_CODE_LEN];

    for i in 0..(USIZE_SIZE + ECC_CODE_LEN) * 8 {
        let x = (i as u32) % width;
        let y = (i as u32) / width;
        let pixel = img.get_pixel(x, y);
        let bit = pixel[chan] & 0x1;
        block[i / 8] |= (bit << (i % 8)) as u8;
        chan = (chan + 1) % 3;
    }
    // println!("len block: {:X?}", block);
    let decoded_len_block = decode_ecc(vec![block])?;

    let data_len_bytes = &decoded_len_block[0..USIZE_SIZE];
    let data_len = LittleEndian::read_uint(data_len_bytes, USIZE_SIZE) as usize;

    let num_blocks = (data_len + ECC_DATA_LEN - 1) / ECC_DATA_LEN;

    // block regions determined by num_blocks + 1, not num_blocks bc data len block
    let block_region_size = width * height / (num_blocks + 1) as u32;

    // get blocks
    let mut blocks = Vec::with_capacity(num_blocks);
    let mut data_left = data_len;
    for i in 1..num_blocks + 1 {
        let block_start = (i as u32) * block_region_size;
        let block_data_len = if data_left > ECC_DATA_LEN {
            ECC_DATA_LEN
        } else {
            data_left
        };
        let mut block = vec![0; block_data_len + ECC_CODE_LEN];
        let mut chan = 0;
        for j in 0..(block_data_len + ECC_CODE_LEN) * 8 {
            let x = ((j as u32) + block_start) % width;
            let y = ((j as u32) + block_start) / width;
            let pixel = img.get_pixel_mut(x as u32, y as u32);
            let bit = (pixel[chan] & 0x1) as u8;
            block[j / 8] |= bit << (j % 8);
            chan = (chan + 1) % 3;
        }
        blocks.push(block);
        data_left -= block_data_len;
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
    blocks.push(encoder.encode(&(input.len() as u64).to_le_bytes()));
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

fn decode_ecc(blocks: Vec<Vec<u8>>) -> Result<Vec<u8>, Error> {
    let decoder = reed_solomon::Decoder::new(ECC_CODE_LEN);
    let mut buf = Vec::new();
    for block in blocks.into_iter() {
        let decoded = decoder
            .correct(&block[..], None)
            .map_err(|e| anyhow!(format!("message data is corrupted: {:?}", e)))?;
        buf.extend_from_slice(decoded.data());
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::{decode_img_inner, encode_img_inner};
    use std::fs::File;
    use std::io::prelude::*;

    fn test_sporkmarmot(data: &[u8]) {
        let mut buf = Vec::new();
        let mut f = File::open("./pkg/examples/sporkmarmot_riding_bufficorn.png").unwrap();
        f.read_to_end(&mut buf).unwrap();

        let steg = encode_img_inner(&buf, data).unwrap();
        let decoded = decode_img_inner(&steg).unwrap();

        assert_eq!(&data[..], decoded);
    }

    #[test]
    fn test_hello_world() {
        test_sporkmarmot(b"hello world!");
    }

    #[test]
    fn test_various_lengths() {
        let data = b"my hovercraft is full of eels!";
        test_sporkmarmot(&data[..]);

        let data = b"Victorious warriors win first and then go to war, while defeated warriors go \
        to war first and then seek to win";
        test_sporkmarmot(&data[..]);

        let data = b"Governments of the Industrial World, you weary giants of flesh and steel, I come from Cyberspace, the new home of Mind. On behalf of the future, I ask you of the past to leave us alone. You are not welcome among us. You have no sovereignty where we gather.\r\n\r\nWe have no elected government, nor are we likely to have one, so I address you with no greater authority than that with which liberty itself always speaks. I declare the global social space we are building to be naturally independent of the tyrannies you seek to impose on us. You have no moral right to rule us nor do you possess any methods of enforcement we have true reason to fear.\r\n\r\nGovernments derive their just powers from the consent of the governed. You have neither solicited nor received ours. We did not invite you. You do not know us, nor do you know our world. Cyberspace does not lie within your borders. Do not think that you can build it, as though it were a public construction project. You cannot. It is an act of nature and it grows itself through our collective actions.\r\n\r\nYou have not engaged in our great and gathering conversation, nor did you create the wealth of our marketplaces. You do not know our culture, our ethics, or the unwritten codes that already provide our society more order than could be obtained by any of your impositions.\r\n\r\nYou claim there are problems among us that you need to solve. You use this claim as an excuse to invade our precincts. Many of these problems don\'t exist. Where there are real conflicts, where there are wrongs, we will identify them and address them by our means. We are forming our own Social Contract. This governance will arise according to the conditions of our world, not yours. Our world is different.\r\n\r\nCyberspace consists of transactions, relationships, and thought itself, arrayed like a standing wave in the web of our communications. Ours is a world that is both everywhere and nowhere, but it is not where bodies live.\r\n\r\nWe are creating a world that all may enter without privilege or prejudice accorded by race, economic power, military force, or station of birth.\r\n\r\nWe are creating a world where anyone, anywhere may express his or her beliefs, no matter how singular, without fear of being coerced into silence or conformity.\r\n\r\nYour legal concepts of property, expression, identity, movement, and context do not apply to us. They are all based on matter, and there is no matter here.\r\n\r\nOur identities have no bodies, so, unlike you, we cannot obtain order by physical coercion. We believe that from ethics, enlightened self-interest, and the commonweal, our governance will emerge. Our identities may be distributed across many of your jurisdictions. The only law that all our constituent cultures would generally recognize is the Golden Rule. We hope we will be able to build our particular solutions on that basis. But we cannot accept the solutions you are attempting to impose.\r\n\r\nIn the United States, you have today created a law, the Telecommunications Reform Act, which repudiates your own Constitution and insults the dreams of Jefferson, Washington, Mill, Madison, DeToqueville, and Brandeis. These dreams must now be born anew in us.\r\n\r\nYou are terrified of your own children, since they are natives in a world where you will always be immigrants. Because you fear them, you entrust your bureaucracies with the parental responsibilities you are too cowardly to confront yourselves. In our world, all the sentiments and expressions of humanity, from the debasing to the angelic, are parts of a seamless whole, the global conversation of bits. We cannot separate the air that chokes from the air upon which wings beat.\r\n\r\nIn China, Germany, France, Russia, Singapore, Italy and the United States, you are trying to ward off the virus of liberty by erecting guard posts at the frontiers of Cyberspace. These may keep out the contagion for a small time, but they will not work in a world that will soon be blanketed in bit-bearing media.\r\n\r\nYour increasingly obsolete information industries would perpetuate themselves by proposing laws, in America and elsewhere, that claim to own speech itself throughout the world. These laws would declare ideas to be another industrial product, no more noble than pig iron. In our world, whatever the human mind may create can be reproduced and distributed infinitely at no cost. The global conveyance of thought no longer requires your factories to accomplish.\r\n\r\nThese increasingly hostile and colonial measures place us in the same position as those previous lovers of freedom and self-determination who had to reject the authorities of distant, uninformed powers. We must declare our virtual selves immune to your sovereignty, even as we continue to consent to your rule over our bodies. We will spread ourselves across the Planet so that no one can arrest our thoughts.\r\n\r\nWe will create a civilization of the Mind in Cyberspace. May it be more humane and fair than the world your governments have made before.";
        test_sporkmarmot(&data[..]);
    }
}
