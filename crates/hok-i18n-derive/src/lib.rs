use proc_macro::TokenStream;
use quote::quote;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use syn::{Data, DeriveInput, Fields, Item, Meta, MetaList, Type, parse_macro_input};

#[proc_macro_derive(I18nHelp)]
pub fn derive_i18n_help(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let hok_root =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    // ── Extract global arg names from Cli struct ──────────────────────────
    let mut global_args: Vec<String> = vec![];
    if let Data::Struct(ref ds) = input.data
        && let Fields::Named(ref nf) = ds.fields {
            for field in &nf.named {
                let name = match &field.ident {
                    Some(i) => i.to_string(),
                    None => continue,
                };
                if name == "command" || name == "verbose" {
                    continue;
                }
                global_args.push(name);
            }
        }
    // Always-patched auto-generated / flattened args
    for extra in &["verbose", "quiet"] {
        if !global_args.contains(&extra.to_string()) {
            global_args.push(extra.to_string());
        }
    }

    // ── Read mod.rs for subcommand enum ───────────────────────────────────
    let mod_src = read_file(&hok_root.join("src/cmd/mod.rs"));
    let mod_file = syn::parse_file(&mod_src).expect("parse src/cmd/mod.rs");
    let cmd_enum = mod_file
        .items
        .iter()
        .find_map(|i| {
            if let Item::Enum(e) = i {
                if e.ident == "Command" {
                    Some(e)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("enum Command not found");

    let mut cmds: Vec<(String, bool, String, String)> = vec![];
    for var in &cmd_enum.variants {
        let p = var.ident.to_string();
        // The subcommand's module/file name comes from its single `Args`
        // field type path (e.g. `CiAutoPr(auto_pr::Args)` -> `auto_pr`), which is
        // authoritative: the variant ident is not always a match (e.g. the
        // `ci-auto-pr` command lives in `auto_pr.rs`).
        let module = extract_module(&var.fields);
        let fn_ = if module.is_empty() {
            to_file_name(&p)
        } else {
            module
        };
        let yk = if hok_root.join(format!("src/cmd/{}.rs", fn_)).exists() {
            fn_.clone()
        } else {
            fn_.replace('_', "")
        };
        cmds.push((
            to_kebab(&p),
            matches!(var.fields, Fields::Unnamed(ref u) if u.unnamed.len() == 1),
            fn_,
            yk,
        ));
    }

    // Top-level args of every command (including the ones that only group
    // subcommands, e.g. `shim -g`), plus the per-subcommand args of the latter.
    let mut plain: HashMap<String, Vec<String>> = HashMap::new();
    let mut nested: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();

    for (_kebab, has, fname, ykey) in &cmds {
        if !has {
            continue;
        }
        let mut p = hok_root.join(format!("src/cmd/{}.rs", fname));
        if !p.exists() {
            p = hok_root.join(format!("src/cmd/{}.rs", fname.replace('_', "")));
        }
        if !p.exists() {
            continue;
        }
        let src = read_file(&p);
        let file = syn::parse_file(&src).unwrap_or_else(|_| panic!("parse src/cmd/{}.rs", fname));
        let s = file.items.iter().find_map(|i| {
            if let Item::Struct(s) = i {
                if s.ident == "Args" {
                    Some(s)
                } else {
                    None
                }
            } else {
                None
            }
        });
        let Some(s) = s else { continue };
        let is_sub = s.fields.iter().any(is_subcommand_field);
        // A command that groups subcommands still has args of its own (e.g.
        // `shim -g/--global`); they must be patched exactly like a leaf
        // command's, otherwise clap falls back to the English doc comment.
        let fields = extract_field_names(&s.fields);
        if !fields.is_empty() {
            plain.insert(ykey.clone(), fields);
        }
        if is_sub {
            let inner = file.items.iter().find_map(|i| {
                if let Item::Enum(e) = i {
                    if e.ident == "Command" {
                        Some(e)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            if let Some(inner) = inner {
                let mut subs = vec![];
                for var in &inner.variants {
                    let pk = var.ident.to_string();
                    subs.push((to_kebab(&pk), extract_field_names(&var.fields)));
                }
                nested.insert(ykey.clone(), subs);
            }
        }
    }

    // ── Generate root-arg patches ─────────────────────────────────────────
    let heading_options = "cli_help.heading.options";
    let global_patches: Vec<_> = global_args.iter().map(|name| {
        let tk = format!("cli_help.hok.args.{name}");
        quote! { .mut_arg(#name, |a| {
            let fallback = a.get_help().map(|h| h.to_string()).unwrap_or_default();
            a.help(i18n_cli_text(#tk, &fallback))
                .help_heading(::hok_i18n::t!(#heading_options).to_string())
        }) }
    }).collect();

    // ── Generate subcommand patches ───────────────────────────────────────
    let usage_t = "cli_help.heading.usage";
    let sub_h = "cli_help.heading.subcommands";
    let root_tmpl = quote! {
        format!(
            "{{before-help}}{{name}} {{version}}\n{{author-with-newline}}{about}\n{usage} {{usage}}\n\n{{all-args}}{{after-help}}",
            about = ::hok_i18n::t!("cli_help.hok.about"),
            usage = ::hok_i18n::t!(#usage_t),
        )
    };
    let sub_tmpl = quote! {
        format!(
            "{{before-help}}{{name}}\n{{about-with-newline}}\n{usage} {{usage}}\n\n{{all-args}}{{after-help}}",
            usage = ::hok_i18n::t!(#usage_t),
        )
    };

    let mut sub_patches = vec![];
    for (kebab, _, _, ykey) in &cmds {
        let ta = format!("cli_help.{ykey}.about");
        let tla = format!("cli_help.{ykey}.long_about");
        let mut ap = vec![];
        if let Some(fs) = plain.get(ykey.as_str()) {
            for fn_ in fs {
                let tk = format!("cli_help.{ykey}.args.{fn_}");
                ap.push(quote! { .mut_arg(#fn_, |a| {
                    let fallback = a.get_help().map(|h| h.to_string()).unwrap_or_default();
                    a.help(i18n_cli_text(#tk, &fallback))
                }) });
            }
        }
        if let Some(subs) = nested.get(ykey.as_str()) {
            for (sk, sfs) in subs {
                let tsa = format!("cli_help.{ykey}.args.{sk}");
                let tla_sk = format!("cli_help.{ykey}.args.{sk}.long_about");
                let sa: Vec<_> = sfs
                    .iter()
                    .map(|fn_| {
                        let tk = format!("cli_help.{ykey}.args.{sk}.{fn_}");
                        quote! { .mut_arg(#fn_, |a| {
                            let fallback = a.get_help().map(|h| h.to_string()).unwrap_or_default();
                            a.help(i18n_cli_text(#tk, &fallback))
                        }) }
                    })
                    .collect();
                ap.push(quote! { .mut_subcommand(#sk, |s| {
                    let about = s.get_about().map(|h| h.to_string()).unwrap_or_default();
                    let long = s.get_long_about().map(|h| h.to_string()).unwrap_or_default();
                    s.help_template(#sub_tmpl)
                        .about(i18n_cli_text(#tsa, &about))
                        .long_about(i18n_cli_long_about(#tla_sk, #tsa, &long))
                        #(#sa)*
                }) });
            }
        }
        sub_patches.push(quote! { .mut_subcommand(#kebab, |s| {
            let about = s.get_about().map(|h| h.to_string()).unwrap_or_default();
            let long = s.get_long_about().map(|h| h.to_string()).unwrap_or_default();
            s.help_template(#sub_tmpl)
                .subcommand_help_heading(::hok_i18n::t!(#sub_h).to_string())
                .about(i18n_cli_text(#ta, &about))
                .long_about(i18n_cli_long_about(#tla, #ta, &long))
                #(#ap)*
        }) });
    }

    let expanded = quote! {
        /// Render the localized help text for `key`, falling back to the text
        /// clap derived from the doc comment when the key is missing — a raw
        /// `cli_help.*` key must never reach the user.
        fn i18n_cli_text(key: &str, fallback: &str) -> String {
            let v = ::hok_i18n::t!(key);
            if v.starts_with("cli_help.") {
                fallback.to_string()
            } else {
                v
            }
        }

        /// Long help has its own key only for the commands that carry a real
        /// long description. Everything else reuses the translated short
        /// `about` (falling back to clap's English text only if that key is
        /// missing too), so `--help` never silently degrades to English.
        fn i18n_cli_long_about(key: &str, fallback_key: &str, clap_text: &str) -> String {
            let v = ::hok_i18n::t!(key);
            if !v.starts_with("cli_help.") {
                return v;
            }
            let about = ::hok_i18n::t!(fallback_key);
            if about.starts_with("cli_help.") {
                clap_text.to_string()
            } else {
                about
            }
        }

        impl #struct_name {
            fn patch_i18n(cmd: ::clap::Command) -> ::clap::Command {
                cmd
                    .about(::hok_i18n::t!("cli_help.hok.about").to_string())
                    .after_help(::hok_i18n::t!("cli_help.after_help").to_string())
                    .subcommand_help_heading(::hok_i18n::t!("cli_help.heading.subcommands").to_string())
                    .help_template(#root_tmpl)
                    #(#global_patches)*
                    #(#sub_patches)*
            }
        }
    };

    TokenStream::from(expanded)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn read_file(p: &PathBuf) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("hok-i18n-derive: {:?}: {e}", p))
}

fn to_kebab(s: &str) -> String {
    let mut r = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            r.push('-');
        }
        r.push(c.to_ascii_lowercase());
    }
    r
}

fn to_file_name(s: &str) -> String {
    let mut r = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            r.push('_');
        }
        r.push(c.to_ascii_lowercase());
    }
    r
}

/// Extract the module name from a `Name::Args`-shaped variant field type
/// (e.g. `auto_pr::Args` -> `auto_pr`); returns empty when the variant has no
/// such field, in which case the caller falls back to the variant ident.
fn extract_module(fields: &Fields) -> String {
    if let Fields::Unnamed(u) = fields
        && let Some(first) = u.unnamed.first()
        && let Type::Path(tp) = &first.ty
        && let Some(seg) = tp.path.segments.first()
    {
        seg.ident.to_string()
    } else {
        String::new()
    }
}

fn extract_field_names(fields: &Fields) -> Vec<String> {
    const SKIP: &[&str] = &["subcommand", "args"];
    match fields {
        Fields::Named(n) => n
            .named
            .iter()
            .filter(|f| !is_subcommand_field(f))
            .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
            .filter(|n| !SKIP.contains(&n.as_str()))
            .collect(),
        _ => vec![],
    }
}

/// Whether a field is the `#[command(subcommand)]` (or `#[clap(subcommand)]`)
/// slot holding the nested command enum. It is not a CLI argument of its own,
/// so it must not be looked up as one.
fn is_subcommand_field(f: &syn::Field) -> bool {
    f.attrs.iter().any(|a| {
        if let Meta::List(MetaList { path, tokens, .. }) = &a.meta {
            (path.is_ident("command") || path.is_ident("clap"))
                && tokens.to_string().contains("subcommand")
        } else {
            false
        }
    })
}
