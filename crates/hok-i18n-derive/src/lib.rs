use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Item, Meta, MetaList};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

#[proc_macro_derive(I18nHelp)]
pub fn derive_i18n_help(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let hok_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    // ── Extract global arg names from Cli struct ──────────────────────────
    let mut global_args: Vec<String> = vec![];
    if let Data::Struct(ref ds) = input.data {
        if let Fields::Named(ref nf) = ds.fields {
            for field in &nf.named {
                let name = match &field.ident { Some(i) => i.to_string(), None => continue };
                if name == "command" || name == "verbose" { continue; }
                global_args.push(name);
            }
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
    let cmd_enum = mod_file.items.iter().find_map(|i| if let Item::Enum(e) = i { if e.ident == "Command" { Some(e) } else { None } } else { None })
        .expect("enum Command not found");

    let mut cmds: Vec<(String, bool, String, String)> = vec![];
    for var in &cmd_enum.variants {
        let p = var.ident.to_string();
        let fn_ = to_file_name(&p);
        let yk = if hok_root.join(format!("src/cmd/{}.rs", fn_)).exists() { fn_.clone() } else { fn_.replace('_', "") };
        cmds.push((to_kebab(&p), matches!(var.fields, Fields::Unnamed(ref u) if u.unnamed.len() == 1), fn_, yk));
    }

    let mut plain: HashMap<String, Vec<String>> = HashMap::new();
    let mut nested: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();

    for (_kebab, has, fname, ykey) in &cmds {
        if !has { continue; }
        let mut p = hok_root.join(format!("src/cmd/{}.rs", fname));
        if !p.exists() { p = hok_root.join(format!("src/cmd/{}.rs", fname.replace('_', ""))); }
        if !p.exists() { continue; }
        let src = read_file(&p);
        let file = syn::parse_file(&src).unwrap_or_else(|_| panic!("parse src/cmd/{}.rs", fname));
        let s = file.items.iter().find_map(|i| if let Item::Struct(s) = i { if s.ident == "Args" { Some(s) } else { None } } else { None });
        let Some(s) = s else { continue };
        let is_sub = s.fields.iter().any(|f| {
            f.attrs.iter().any(|a| {
                if let Meta::List(MetaList { path, tokens, .. }) = &a.meta {
                    path.is_ident("command") && tokens.to_string().contains("subcommand")
                } else { false }
            })
        });
        if is_sub {
            let inner = file.items.iter().find_map(|i| if let Item::Enum(e) = i { if e.ident == "Command" { Some(e) } else { None } } else { None });
            if let Some(inner) = inner {
                let mut subs = vec![];
                for var in &inner.variants {
                    let pk = var.ident.to_string();
                    subs.push((to_kebab(&pk), extract_field_names(&var.fields)));
                }
                nested.insert(ykey.clone(), subs);
            }
        } else {
            let fields = extract_field_names(&s.fields);
            if !fields.is_empty() { plain.insert(ykey.clone(), fields); }
        }
    }

    // ── Generate root-arg patches ─────────────────────────────────────────
    let heading_options = "cli_help.heading.options";
    let global_patches: Vec<_> = global_args.iter().map(|name| {
        let tk = format!("cli_help.hok.args.{name}");
        quote! { .mut_arg(#name, |a| a.help(t!(#tk).to_string()).help_heading(t!(#heading_options).to_string())) }
    }).collect();

    // ── Generate subcommand patches ───────────────────────────────────────
    let usage_t = "cli_help.heading.usage";
    let sub_h = "cli_help.heading.subcommands";
    let root_tmpl = quote! {
        format!(
            "{{before-help}}{{name}} {{version}}\n{{author-with-newline}}{about}\n{usage} {{usage}}\n\n{{all-args}}{{after-help}}",
            about = t!("cli_help.hok.about"),
            usage = t!(#usage_t),
        )
    };
    let sub_tmpl = quote! {
        format!(
            "{{before-help}}{{name}}\n{{about-with-newline}}\n{usage} {{usage}}\n\n{{all-args}}{{after-help}}",
            usage = t!(#usage_t),
        )
    };

    let mut sub_patches = vec![];
    for (kebab, _, _, ykey) in &cmds {
        let ta = format!("cli_help.{ykey}.about");
        let mut ap = vec![];
        if let Some(fs) = plain.get(ykey.as_str()) {
            for fn_ in fs {
                let tk = format!("cli_help.{ykey}.args.{fn_}");
                ap.push(quote! { .mut_arg(#fn_, |a| a.help(t!(#tk).to_string())) });
            }
        }
        if let Some(subs) = nested.get(ykey.as_str()) {
            for (sk, sfs) in subs {
                let tsa = format!("cli_help.{ykey}.args.{sk}");
                let sa: Vec<_> = sfs.iter().map(|fn_| {
                    let tk = format!("cli_help.{ykey}.args.{sk}.{fn_}");
                    quote! { .mut_arg(#fn_, |a| a.help(t!(#tk).to_string())) }
                }).collect();
                ap.push(quote! { .mut_subcommand(#sk, |s| s
                    .help_template(#sub_tmpl)
                    .about(t!(#tsa).to_string())
                    .long_about(t!(#tsa).to_string())
                    #(#sa)*
                ) });
            }
        }
        sub_patches.push(quote! { .mut_subcommand(#kebab, |s| s
            .help_template(#sub_tmpl)
            .subcommand_help_heading(t!(#sub_h).to_string())
            .about(t!(#ta).to_string())
            .long_about(t!(#ta).to_string())
            #(#ap)*
        ) });
    }

    let expanded = quote! {
        impl #struct_name {
            fn patch_i18n(cmd: ::clap::Command) -> ::clap::Command {
                cmd
                    .about(t!("cli_help.hok.about").to_string())
                    .after_help(t!("cli_help.after_help").to_string())
                    .subcommand_help_heading(t!("cli_help.heading.subcommands").to_string())
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
    for (i, c) in s.char_indices() { if c.is_uppercase() && i > 0 { r.push('-'); } r.push(c.to_ascii_lowercase()); }
    r
}

fn to_file_name(s: &str) -> String {
    let mut r = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() { if c.is_uppercase() && i > 0 { r.push('_'); } r.push(c.to_ascii_lowercase()); }
    r
}

fn extract_field_names(fields: &Fields) -> Vec<String> {
    const SKIP: &[&str] = &["subcommand", "args"];
    match fields {
        Fields::Named(n) => n.named.iter()
            .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
            .filter(|n| !SKIP.contains(&n.as_str()))
            .collect(),
        _ => vec![],
    }
}
