// vt-screen.js — replay a captured VT stream into screen snapshots.
//
//   node vt-screen.js <capture.bin> [cols] [rows] [maxFrames]
//
// Handles the subset ConPTY + indicatif emit: printable text, CR/LF/BS, CSI
// cursor movement (A/B/C/D/G/H/f), erase (J/K), and ignores SGR/mode/OSC
// sequences. Prints evenly spaced intermediate frames plus the final screen, so
// a captured TTY run can be read like a terminal recording.
'use strict';
const fs = require('fs');

const file = process.argv[2];
const cols = Number(process.argv[3] || 100);
const rows = Number(process.argv[4] || 20);
const maxFrames = Number(process.argv[5] || 6);

const buf = fs.readFileSync(file);
let screen = Array.from({ length: rows }, () => new Array(cols).fill(' '));
let r = 0;
let c = 0;
const frames = [];
let lastKey = '';

const text = (row) => row.join('').replace(/\s+$/, '');

function snapshot() {
    const key = screen.map(text).join('\n') + '|' + r + '|' + c;
    if (key === lastKey) return;
    lastKey = key;
    frames.push({ lines: screen.map(text), r, c });
}

function clamp() {
    if (c < 0) c = 0;
    if (c > cols) c = cols;
    if (r < 0) r = 0;
    if (r >= rows) r = rows - 1;
}

/// Display width in terminal columns (CJK and full-width forms take two).
function displayWidth(ch) {
    const cp = ch.codePointAt(0);
    const wide =
        (cp >= 0x1100 && cp <= 0x115f) ||
        (cp >= 0x2e80 && cp <= 0x303e) ||
        (cp >= 0x3041 && cp <= 0x33ff) ||
        (cp >= 0x3400 && cp <= 0x4dbf) ||
        (cp >= 0x4e00 && cp <= 0x9fff) ||
        (cp >= 0xa000 && cp <= 0xa4cf) ||
        (cp >= 0xac00 && cp <= 0xd7a3) ||
        (cp >= 0xf900 && cp <= 0xfaff) ||
        (cp >= 0xfe30 && cp <= 0xfe6f) ||
        (cp >= 0xff00 && cp <= 0xff60) ||
        (cp >= 0xffe0 && cp <= 0xffe6) ||
        cp >= 0x20000;
    return wide ? 2 : 1;
}

function newline() {
    r += 1;
    if (r >= rows) {
        screen.shift();
        screen.push(new Array(cols).fill(' '));
        r = rows - 1;
    }
}

let i = 0;
while (i < buf.length) {
    const ch = buf[i];
    if (ch === 0x1b) {
        const next = buf[i + 1];
        if (next === 0x5b) {                       // CSI
            let j = i + 2;
            let params = '';
            while (j < buf.length && !(buf[j] >= 0x40 && buf[j] <= 0x7e)) {
                params += String.fromCharCode(buf[j]);
                j += 1;
            }
            const final = String.fromCharCode(buf[j] ?? 0x40);
            const nums = params.replace(/^[?>=]/, '').split(';').filter((s) => s !== '');
            const n = (idx, dflt) => (nums[idx] === undefined ? dflt : Number(nums[idx]));
            switch (final) {
                case 'A': r -= n(0, 1); break;
                case 'B': r += n(0, 1); break;
                case 'C': c += n(0, 1); break;
                case 'D': c -= n(0, 1); break;
                case 'G': c = n(0, 1) - 1; break;
                case 'H': case 'f': r = n(0, 1) - 1; c = n(1, 1) - 1; break;
                case 'J': {
                    const mode = n(0, 0);
                    if (mode === 2) screen = Array.from({ length: rows }, () => new Array(cols).fill(' '));
                    else if (mode === 0) {
                        for (let x = c; x < cols; x += 1) screen[r][x] = ' ';
                        for (let y = r + 1; y < rows; y += 1) screen[y].fill(' ');
                    } else {
                        for (let x = 0; x <= c; x += 1) screen[r][x] = ' ';
                        for (let y = 0; y < r; y += 1) screen[y].fill(' ');
                    }
                    break;
                }
                case 'K': {
                    const mode = n(0, 0);
                    if (mode === 0) for (let x = c; x < cols; x += 1) screen[r][x] = ' ';
                    else if (mode === 1) for (let x = 0; x <= c; x += 1) screen[r][x] = ' ';
                    else screen[r].fill(' ');
                    break;
                }
                default: break;                        // SGR (m), modes (h/l), etc.
            }
            clamp();
            i = j + 1;
            continue;
        }
        if (next === 0x5d) {                       // OSC ... BEL/ST
            let j = i + 2;
            while (j < buf.length && buf[j] !== 0x07) j += 1;
            i = j + 1;
            continue;
        }
        i += 2;
        continue;
    }
    if (ch === 0x0d) { c = 0; i += 1; continue; }
    if (ch === 0x0a) { newline(); i += 1; continue; }
    if (ch === 0x08) { c -= 1; clamp(); i += 1; continue; }
    if (ch === 0x07) { i += 1; continue; }
    // A UTF-8 code point: decode it so CJK/braille land as one cell each.
    const cp = buf[i];
    let size = 1;
    if (cp >= 0xf0) size = 4;
    else if (cp >= 0xe0) size = 3;
    else if (cp >= 0xc0) size = 2;
    const ch2 = buf.toString('utf8', i, i + size);
    const w = displayWidth(ch2);
    // ConPTY pads every row to the console width and relies on the terminal's
    // auto-wrap, so the emulator must wrap too (a wide char never splits).
    if (c + w > cols) {
        c = 0;
        newline();
    }
    screen[r][c] = ch2;
    if (w === 2 && c + 1 < cols) screen[r][c + 1] = '';
    c += w;
    i += size;
    snapshot();
}

snapshot();
const shown = [];
if (frames.length > maxFrames) {
    // Keep the first frames (start-up) and the last ones (settling).
    const first = Math.ceil(maxFrames / 2);
    for (let k = 0; k < first; k += 1) shown.push(frames[Math.floor((k * frames.length) / maxFrames)]);
    for (let k = first; k < maxFrames; k += 1) {
        shown.push(frames[frames.length - (maxFrames - k)]);
    }
} else {
    shown.push(...frames);
}

console.log(`frames=${frames.length} shown=${shown.length} cols=${cols} rows=${rows}`);
shown.forEach((f, idx) => {
    console.log(`\n--- frame ${idx + 1}/${shown.length} (cursor ${f.r + 1},${f.c + 1}) ---`);
    const body = f.lines.join('\n').replace(/\n+$/, '');
    console.log(body.length ? body : '(empty)');
});
