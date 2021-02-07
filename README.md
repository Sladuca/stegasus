## Stegasus

Stegasus is WASM-focused steganography library written in rust. Compared to
[steganography](https://github.com/teovoinea/steganography), it has two main
differences:

1. The interface is wasm-friendly, notably focused on using byte buffers as the
primary method of consuming image data.
2. Though it still uses LSB steganography, Stegasus differs in that:
  * Stegasus pre-processes input data with reed-solomon error-correcting codes 
  in an attempt to keep things simple as a prototype but add a small 
  degree of robustness.
  * Instead of encoding the data right into the first pixels of the image, 
  stegasus breaks the data into multiple 255-bit ECC blocks and distributes those
  blocks throughout the image.

## Usage

Stegasus was built primarily as a part of 
[furball_dapp](https://github.com/simondpalmer/furball_dapp) during 
[EthDenver](https://www.ethdenver.com/).
