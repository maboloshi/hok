#!/usr/bin/env python3
"""Extract clap help texts from src/cmd/*.rs

Usage:
  python scripts/extract_cli_help.py              # Print YAML to stdout
  python scripts/extract_cli_help.py --check      # Dry-run: list parsed files/fields
  python scripts/extract_cli_help.py --to-file    # Append to locales/en.yml
"""

import argparse
import os
import re
import sys
from collections import OrderedDict
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CMD_DIR = REPO_ROOT / "src" / "cmd"
LOCALE_EN = REPO_ROOT / "locales" / "en.yml"
LOCALE_ZH = REPO_ROOT / "locales" / "zh.yml"


# ── Helpers ─────────────────────────────────────────────────────────────────


def get_doc_comments(lines, start_idx):
    """Walk backwards from start_idx collecting /// doc comment lines."""
    parts = []
    j = start_idx
    while j >= 0:
        line = lines[j]
        m = re.match(r'^\s*///\s?(.*)', line)
        if m:
            parts.insert(0, m.group(1).replace('\r', ''))
            j -= 1
        elif re.match(r'^\s*(#\[|$|//!)', line):
            j -= 1
        else:
            break
    text = ' '.join(parts).strip()
    return text


def get_aliases_from_line(line):
    """Extract alias strings from a #[clap(...)] or #[command(...)] line."""
    return re.findall(r'(?:alias\s*=\s*)"([^"]+)"', line)


def find_field_name(lines, start_idx):
    """Find field name on or after start_idx.  Returns (name, end_idx) or (None, start_idx).

    The regex uses (?!:) to avoid matching type annotations like ArgAction:: or clap::.
    """
    # inline on same line as the attribute
    m = re.search(r'(?:pub\s+)?(\w+)\s*:(?!:)\s*', lines[start_idx])
    if m:
        return m.group(1), start_idx

    for k in range(start_idx + 1, len(lines)):
        line = lines[k]
        if re.match(r'^\s*(#|//)', line):
            continue
        if not line.strip():
            continue
        m = re.search(r'(?:pub\s+)?(\w+)\s*:(?!:)\s*', line)
        if m:
            return m.group(1), k
        # closing brace or bracket — give up
        if re.match(r'^\s*[}\]]', line):
            break
    return None, start_idx


def find_matching_brace(lines, brace_idx):
    """Return line index of matching '}' for '{' at brace_idx."""
    depth = 0
    for k in range(brace_idx, len(lines)):
        # strip line comments
        line = re.sub(r'//.*', '', lines[k])
        for ch in line:
            if ch == '{':
                depth += 1
            elif ch == '}':
                depth -= 1
                if depth == 0:
                    return k
    return len(lines) - 1


def parse_fields_in_block(lines, start_idx, end_idx):
    """Parse #[arg/clap/command] fields inside { … } block. Returns {name: help_text}."""
    fields = OrderedDict()
    i = start_idx
    while i <= end_idx:
        if re.search(r'#\s*\[(?:arg|clap|command)\s*\(', lines[i]):
            doc = get_doc_comments(lines, i - 1)
            fname, i2 = find_field_name(lines, i)
            if fname and doc:
                fields[fname] = doc
            i = i2
        i += 1
    return fields


def yaml_quote(s):
    """Quote a string for YAML if it contains special characters."""
    if not s:
        return '""'
    if re.search(r'[:#"{}\[\],&\*!|>%@`]', s) or s.startswith((' ', '- ')):
        return f'"{s}"'
    return s


def emit_yaml(data, alias_map, file=sys.stdout):
    """Write YAML to *file* (default stdout)."""
    print("cli_help:", file=file)

    for cmd_name in sorted(data, key=str.lower):
        cmd = data[cmd_name]
        print(f"  {cmd_name}:", file=file)
        if cmd.get("about"):
            print(f"    about: {yaml_quote(cmd['about'])}", file=file)
        args = cmd.get("args")
        if args:
            # Separate regular args from subcommand-only entries (those with prefix that matches a variant)
            print(f"    args:", file=file)
            for arg_name in sorted(args, key=str.lower):
                print(f"      {arg_name}: {yaml_quote(args[arg_name])}", file=file)

    if alias_map:
        print(file=file)
        print("# Command aliases (for shell completions)", file=file)
        print("cli_alias:", file=file)
        for key in sorted(alias_map, key=str.lower):
            val = alias_map[key]
            if isinstance(val, dict):
                print(f"  {key}:", file=file)
                for sub in sorted(val, key=str.lower):
                    print(f"    {sub}: [{', '.join(val[sub])}]", file=file)
            else:
                print(f"  {key}: [{', '.join(val)}]", file=file)


# ── Parse a single file ─────────────────────────────────────────────────────


def parse_file(path):
    """Parse a cmd/*.rs file, return (cmd_name, about, args, subcommands, aliases)."""
    cmd_name = path.stem
    with open(path, encoding="utf-8") as fh:
        content = fh.read()
    lines = content.split("\n")

    # Locate pub struct Args
    struct_line = -1
    for i, line in enumerate(lines):
        if "pub struct Args" in line:
            struct_line = i
            break
    if struct_line < 0:
        return None

    about = get_doc_comments(lines, struct_line - 1)
    args = OrderedDict()
    subcommands = OrderedDict()  # name → {about, args{name→help}, aliases[]}

    # Detect subcommand pattern: #[command(subcommand)]
    has_subcommand = False
    for i in range(struct_line, len(lines)):
        if re.search(r'#\s*\[command\s*\(\s*subcommand', lines[i]):
            has_subcommand = True
            break

    enum_line = -1
    enum_name = ""
    if has_subcommand:
        for i in range(struct_line, len(lines)):
            m = re.search(r'pub enum\s+(\w+)', lines[i])
            if m:
                enum_line = i
                enum_name = m.group(1)
                break

    if has_subcommand and enum_line >= 0:
        # ── Subcommand enum parsing ────────
        brace_line = None
        for i in range(enum_line, len(lines)):
            if '{' in lines[i]:
                brace_line = i
                break
        if brace_line is None:
            return None
        enum_end = find_matching_brace(lines, brace_line)

        i = brace_line + 1
        while i < enum_end:
            line = lines[i]
            # skip blank/comment lines
            if re.match(r'^\s*(//|#|$)', line):
                i += 1
                continue

            variant_doc = get_doc_comments(lines, i - 1)

            # look up for alias attribute(s) — walk through ALL consecutive #[clap(...)] lines
            variant_aliases = []
            for j in range(i - 1, -1, -1):
                ll = lines[j]
                if re.search(r'#\s*\[(?:clap|command)\(', ll):
                    variant_aliases.extend(get_aliases_from_line(ll))
                elif re.match(r'^\s*(///|$)', ll):
                    continue
                else:
                    break

            # Match variant patterns
            m = re.match(r'^\s*(\w+)\s*\{', line)
            if m:
                vname = m.group(1)
                block_end = find_matching_brace(lines, i)
                block_fields = parse_fields_in_block(lines, i + 1, block_end - 1)
                prefixed = OrderedDict()
                for k, v in block_fields.items():
                    prefixed[f"{vname}.{k}"] = v
                subcommands[vname] = {"about": variant_doc, "args": prefixed, "aliases": variant_aliases}
                i = block_end + 1
            elif re.match(r'^\s*(\w+)\s*\(', line):
                m = re.match(r'^\s*(\w+)', line)
                vname = m.group(1)
                subcommands[vname] = {"about": variant_doc, "args": OrderedDict(), "aliases": variant_aliases}
                i += 1
            elif m := re.match(r'^\s*(\w+)\s*,?\s*$', line):
                vname = m.group(1)
                if vname != enum_name:  # avoid matching enum name
                    subcommands[vname] = {"about": variant_doc, "args": OrderedDict(), "aliases": variant_aliases}
                i += 1
            else:
                i += 1
    else:
        # ── Plain struct fields ────────────
        brace_line = None
        for i in range(struct_line, len(lines)):
            if '{' in lines[i]:
                brace_line = i
                break
        if brace_line is None:
            return None
        struct_end = find_matching_brace(lines, brace_line)

        i = brace_line + 1
        while i < struct_end:
            line = lines[i]
            if re.search(r'#\s*\[(?:arg|clap|command)\s*\(', line):
                doc = get_doc_comments(lines, i - 1)
                fname, i2 = find_field_name(lines, i)
                if fname and doc:
                    args[fname] = doc
                i = i2
            i += 1

    return cmd_name, about, args, subcommands


# ── Second pass: command-level aliases from mod.rs ──────────────────────────


def parse_command_aliases():
    """Extract command-level clap aliases from src/cmd/mod.rs.

    Handles single and multiple aliases on one line, e.g.:
      #[clap(alias = "rm", alias = "remove")]
    """
    mod_path = CMD_DIR / "mod.rs"
    if not mod_path.exists():
        return {}
    with open(mod_path, encoding="utf-8") as fh:
        content = fh.read()
    lines = content.split("\n")

    alias_map = {}
    for i, line in enumerate(lines):
        # Check if this line contains #[clap(...)] or #[command(...)]
        if not re.search(r'#\s*\[(?:clap|command)\(', line):
            continue
        # Extract ALL aliases from the attribute
        aliases = re.findall(r'alias\s*=\s*"([^"]+)"', line)
        if not aliases:
            continue
        # Look forward for the variant name
        for j in range(i + 1, min(len(lines), i + 4)):
            vm = re.match(r'^\s+(\w+)\s*\(', lines[j])
            if vm:
                vname = vm.group(1).lower()
                alias_map.setdefault(vname, []).extend(aliases)
                break
    return alias_map


# ── Main ─────────────────────────────────────────────────────────────────────


def main():
    parser = argparse.ArgumentParser(description="Extract clap help texts from cmd/*.rs")
    parser.add_argument("--check", action="store_true", help="Dry-run: list parsed files and field counts")
    parser.add_argument("--to-file", action="store_true", help="Append cli_help YAML to locales/en.yml")
    args = parser.parse_args()

    files = sorted(CMD_DIR.glob("*.rs"))
    data = OrderedDict()
    alias_map_cmd = parse_command_aliases()
    alias_map_nested = {}  # for subcommand aliases

    for path in files:
        if path.stem == "mod":
            continue
        result = parse_file(path)
        if result is None:
            continue

        cmd_name, about, cmd_args, subcommands = result

        entry = OrderedDict()
        if about:
            entry["about"] = about

        merged_args = OrderedDict()
        merged_args.update(cmd_args)

        # Merge subcommands
        for sv_name, sv_data in subcommands.items():
            merged_args[sv_name] = sv_data["about"]
            for fk, fv in sv_data["args"].items():
                merged_args[fk] = fv
            # Collect subcommand aliases
            if sv_data["aliases"]:
                alias_map_nested.setdefault(cmd_name, {})[sv_name] = sv_data["aliases"]

        if merged_args:
            entry["args"] = merged_args

        if entry:
            data[cmd_name] = entry

    if args.check:
        print("=== Parsed commands ===")
        for cmd_name in data:
            entry = data[cmd_name]
            ac = len(entry.get("args", {}))
            about_preview = entry.get("about", "")
            if len(about_preview) > 50:
                about_preview = about_preview[:50] + "..."
            print(f"  {cmd_name:20s}  args:{ac:2d}  about: {about_preview}")
        print(f"\nTotal: {len(data)} commands")
        if alias_map_cmd or alias_map_nested:
            print("\n=== Aliases ===")
            for key in sorted(alias_map_cmd):
                print(f"  {key} -> {', '.join(alias_map_cmd[key])}")
            for key in sorted(alias_map_nested):
                for sub in sorted(alias_map_nested[key]):
                    print(f"  {key}.{sub} -> {', '.join(alias_map_nested[key][sub])}")
        return

    if args.to_file:
        with open(LOCALE_EN, "a", encoding="utf-8") as fh:
            fh.write("\n")
            emit_yaml(data, {**alias_map_cmd, **alias_map_nested}, file=fh)
        print(f"Appended to {LOCALE_EN}", file=sys.stderr)
    else:
        emit_yaml(data, {**alias_map_cmd, **alias_map_nested})


if __name__ == "__main__":
    main()
