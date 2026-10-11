// Dump every `--help` surface of the hok CLI, for both locales, into a
// deterministic directory tree so before/after runs can be compared byte-wise.
//
// Usage: node dump-help.js <hok.exe> <out-dir>
'use strict';
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const bin = path.resolve(process.argv[2]);
const outDir = path.resolve(process.argv[3]);

function run(args) {
    // clap renders usage; a broken help surface must still be captured
    // verbatim (stderr + exit code) so before/after stays comparable instead
    // of aborting the dump.
    try {
        return execFileSync(bin, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    } catch (err) {
        const stdout = err.stdout == null ? '' : String(err.stdout);
        const stderr = err.stderr == null ? '' : String(err.stderr);
        return `<<<exit ${err.status}>>>\n${stdout}${stderr}`;
    }
}

/** Parse the `Subcommands:` block of a clap help text into subcommand names. */
function subcommandsOf(help) {
    const lines = help.split(/\r?\n/);
    const start = lines.findIndex((l) => /^Subcommands:/.test(l));
    if (start < 0) return [];
    const names = [];
    for (let i = start + 1; i < lines.length; i++) {
        const line = lines[i];
        if (!line.trim()) break;
        if (!/^\s{2,}/.test(line)) break;
        const m = line.match(/^\s+([a-z0-9][a-z0-9-]*)(?:\s|$)/);
        if (m) names.push(m[1]);
    }
    return names;
}

/** The version line embeds a build timestamp; it is the one allowed diff. */
function normalize(text) {
    return text.replace(/\(built [^)]*\)/g, '(built <ts>)');
}

const LOCALES = ['en', 'zh'];
let count = 0;

for (const locale of LOCALES) {
    const queue = [[]];
    const seen = new Set();
    while (queue.length) {
        const argv = queue.shift();
        const id = argv.join(' ') || '(root)';
        if (seen.has(id)) continue;
        seen.add(id);

        const text = normalize(run(['--lang', locale, ...argv, '--help']));
        const file = path.join(outDir, locale, `${argv.join('_') || 'root'}.txt`);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, text, 'utf8');
        count++;

        for (const sub of subcommandsOf(text)) queue.push([...argv, sub]);
    }
}

console.log(`dumped ${count} help texts into ${outDir}`);
