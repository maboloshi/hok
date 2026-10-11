// Enumerate every real help surface of the hok CLI and report which ones fail.
//
// Usage: node help-audit.js <hok.exe>
'use strict';
const { execFileSync } = require('child_process');
const path = require('path');

const bin = path.resolve(process.argv[2]);
const STDIO = ['ignore', 'pipe', 'pipe'];

function run(args) {
    try {
        const stdout = execFileSync(bin, args, {
            encoding: 'utf8',
            maxBuffer: 64 * 1024 * 1024,
            stdio: STDIO,
        });
        return { code: 0, out: stdout, err: '' };
    } catch (err) {
        return {
            code: err.status == null ? -1 : err.status,
            out: err.stdout == null ? '' : String(err.stdout),
            err: err.stderr == null ? '' : String(err.stderr),
        };
    }
}

/** Subcommand names listed in a successful English `--help` output. */
function subcommandsOf(help) {
    const lines = help.split(/\r?\n/);
    const start = lines.findIndex((l) => /^Subcommands:/.test(l));
    if (start < 0) return [];
    const names = [];
    for (let i = start + 1; i < lines.length; i++) {
        const line = lines[i];
        if (!line.trim()) break;
        if (!/^\s{2,}/.test(line)) break;
        const m = line.match(/^\s+([a-z0-9][a-z0-9-]*)\s{2,}(\S|$)/);
        if (m) names.push(m[1]);
    }
    return names;
}

// Discovery always uses English, because the section headings are localized.
const queue = [[]];
const surfaces = [];
const seen = new Set();
while (queue.length) {
    const argv = queue.shift();
    if (seen.has(argv.join(' '))) continue;
    seen.add(argv.join(' '));
    surfaces.push(argv);
    const help = run(['--lang', 'en', ...argv, '--help']);
    if (help.code !== 0) continue;
    for (const sub of subcommandsOf(help.out)) {
        // clap's `help` subcommand takes a command name, so `--help` after it
        // is not a real surface.
        if (sub === 'help') continue;
        const probe = run(['--lang', 'en', ...argv, sub, '--help']);
        if (probe.code !== 0 && /unrecognized subcommand/.test(probe.err)) continue;
        queue.push([...argv, sub]);
    }
}

const broken = [];
let total = 0;
for (const argv of surfaces) {
    for (const locale of ['en', 'zh']) {
        for (const flag of ['--help', '-h']) {
            total++;
            const r = run(['--lang', locale, ...argv, flag]);
            if (r.code === 0 && r.out.trim()) continue;
            broken.push({
                surface: `hok --lang ${locale} ${argv.join(' ')} ${flag}`.replace('  ', ' '),
                code: r.code,
                message: (r.err || r.out).split('\n').find((l) => l.trim()) || '(empty)',
            });
        }
    }
}

const lines = [
    `surfaces: ${surfaces.length}   probes: ${total}   broken: ${broken.length}`,
    ...broken.map((b) => `  exit ${b.code}  ${b.surface}\n           ${b.message}`),
];
console.log(lines.join('\n'));
