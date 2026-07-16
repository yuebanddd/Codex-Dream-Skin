import { deflateSync } from "node:zlib";

const size = 128;
const pixels = Buffer.alloc((size * 4 + 1) * size);

for (let y = 0; y < size; y += 1) {
  const row = y * (size * 4 + 1);
  pixels[row] = 0;
  for (let x = 0; x < size; x += 1) {
    const offset = row + 1 + x * 4;
    const glow = Math.max(0, 1 - Math.hypot(x - 64, y - 58) / 82);
    pixels[offset] = Math.round(27 + glow * 92);
    pixels[offset + 1] = Math.round(13 + glow * 34);
    pixels[offset + 2] = Math.round(28 + glow * 56);
    pixels[offset + 3] = 255;
  }
}

function crc32(buffer) {
  let value = 0xffffffff;
  for (const byte of buffer) {
    value ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value >>> 1) ^ (0xedb88320 & -(value & 1));
    }
  }
  return (value ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const name = Buffer.from(type);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(Buffer.concat([name, data])));
  return Buffer.concat([length, name, data, checksum]);
}

const header = Buffer.alloc(13);
header.writeUInt32BE(size, 0);
header.writeUInt32BE(size, 4);
header[8] = 8;
header[9] = 6;

const png = Buffer.concat([
  Buffer.from("89504e470d0a1a0a", "hex"),
  chunk("IHDR", header),
  chunk("IDAT", deflateSync(pixels)),
  chunk("IEND", Buffer.alloc(0)),
]);

process.stdout.write(png.toString("base64"));
