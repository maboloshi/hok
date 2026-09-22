// Builds `traversal-rar5.rar`: a valid RAR5 archive whose single stored member
// is named `..\evil.txt`, i.e. it escapes the extraction root.
//
// rars' own Builder refuses to emit such a name (validate_entry_name), and no
// encoder will produce one, so the archive is derived from a WinRAR-made
// archive whose member name is a neutral 11-byte placeholder. Placeholder and
// traversal name are the same length, so every size field stays valid; only the
// name bytes and the covering header CRC-32 change.
//
// Generate the source with quick-open disabled: a quick-open record keeps a
// verbatim copy of the headers, and a reader may extract from that copy
// instead, so editing only the live header would measure nothing.
//
// Usage:
//   Rar.exe a -ma5 -m0 -qo- -ep1 traversal-src.rar abcdefghijk
//   node make-traversal-fixture.cjs traversal-src.rar traversal-rar5.rar
const fs = require('node:fs');

const PLACEHOLDER = 'abcdefghijk';
const TRAVERSAL = '..\\evil.txt';

const crcTable = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
const crc32 = (buf) => {
  let c = -1;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
};

const readVint = (buf, pos) => {
  let value = 0, shift = 0, p = pos;
  for (;;) {
    const b = buf[p++];
    value += (b & 0x7f) * 2 ** shift;
    if ((b & 0x80) === 0) return [value, p];
    shift += 7;
  }
};

const [, , inPath, outPath] = process.argv;
if (PLACEHOLDER.length !== TRAVERSAL.length) throw new Error('name length mismatch');

const buf = Buffer.from(fs.readFileSync(inPath));
if (buf.subarray(0, 8).toString('latin1') !== 'Rar!\x1a\x07\x01\x00') {
  throw new Error('not a RAR5 archive');
}

let patched = false;
let pos = 8;
while (!patched && pos + 4 <= buf.length) {
  const headerStart = pos + 4;         // CRC-32 sits in the 4 bytes before it
  let p = headerStart;

  const [headerSize, afterSize] = readVint(buf, p);
  // `headerSize` counts the header bytes *after* the header-size vint itself,
  // not including it. Verified against WinRAR's own CRC-32: the alternative
  // reading (headerStart + headerSize) fails the stored CRC on every block.
  const headerEnd = afterSize + headerSize;
  p = afterSize;

  const [type, afterType] = readVint(buf, p);
  p = afterType;
  const [flags, afterFlags] = readVint(buf, p);
  p = afterFlags;

  let dataSize = 0;
  if (flags & 0x0001) { [, p] = readVint(buf, p); }   // extra area size
  if (flags & 0x0002) { [dataSize, p] = readVint(buf, p); }

  if (type === 2) {                                    // file header
    const [fileFlags, afterFileFlags] = readVint(buf, p);
    p = afterFileFlags;
    [, p] = readVint(buf, p);                          // unpacked size
    [, p] = readVint(buf, p);                          // attributes
    if (fileFlags & 0x0002) p += 4;                    // mtime
    if (fileFlags & 0x0004) p += 4;                    // data CRC-32
    [, p] = readVint(buf, p);                          // compression info
    [, p] = readVint(buf, p);                          // host OS
    const [nameLen, afterNameLen] = readVint(buf, p);
    const nameAt = afterNameLen;
    const name = buf.subarray(nameAt, nameAt + nameLen).toString('utf8');

    if (name === PLACEHOLDER) {
      Buffer.from(TRAVERSAL, 'utf8').copy(buf, nameAt);
      buf.writeUInt32LE(crc32(buf.subarray(headerStart, headerEnd)), headerStart - 4);
      patched = true;
      break;
    }
  }

  pos = headerEnd + dataSize;
}

if (!patched) throw new Error(`no member named ${PLACEHOLDER} found`);
fs.writeFileSync(outPath, buf);
console.log(`patched -> ${outPath} (member renamed to ${JSON.stringify(TRAVERSAL)})`);
