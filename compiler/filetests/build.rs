//! Load all wat files to structured tests.

use anyhow::Result;
use proc_macro2::Span;
use quote::ToTokens;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use syn::{parse_quote, ExprArray, ExprMatch, Ident, ItemImpl};
use wasm_opt::OptimizationOptions;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wat");

    let item_impl = parse_tests()?;
    fs::write(
        env::var("OUT_DIR")?.parse::<PathBuf>()?.join("tests.rs"),
        item_impl.to_token_stream().to_string(),
    )?;

    Ok(())
}

fn wasm_directory() -> Result<PathBuf> {
    cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .map(|m| m.target_directory.join("wasm32-unknown-unknown").into())
        .map_err(Into::into)
}

/// Read the contents of a directory, returning
/// all wat files.
fn list_wat(dir: impl AsRef<Path>, files: &mut Vec<PathBuf>) -> Result<()> {
    let entry = fs::read_dir(dir)?;
    for entry in entry {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            list_wat(path, files)?;
        } else if path.extension().unwrap_or_default() == "wat" {
            files.push(path);
        }
    }

    Ok(())
}

/// Batch all wat files.
fn wat_files() -> Result<Vec<PathBuf>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("wat");
    let mut files = Vec::new();
    list_wat(&path, &mut files)?;

    let excludes = ["as_if_else.wat"];

    Ok(files
        .into_iter()
        .filter(|f| {
            !excludes.contains(
                &f.file_name()
                    .and_then(|n| n.to_str())
                    .expect("file name not found"),
            )
        })
        .collect())
}

fn examples() -> Result<Vec<PathBuf>> {
    let release = wasm_directory()?.join("release").join("examples");
    if !release.exists() {
        return Ok(Default::default());
    }

    let with_commit_hash = |p: &PathBuf| -> bool {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();

        // for example: addition-6313c94b67ad9699.wasm
        let len = name.len();
        if let Some(index) = name.rfind('-') {
            if len > 22 && index == len - 22 {
                return true;
            }
        }

        false
    };

    let files = fs::read_dir(release)?
        .filter_map(|e| {
            let path = e.ok()?.path();
            if path.extension().unwrap_or_default() == "wasm" && !with_commit_hash(&path) {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for wasm in &files {
        OptimizationOptions::new_opt_level_4()
            .debug_info(false)
            .mvp_features_only()
            .set_converge()
            .run(wasm, wasm)?;
    }

    Ok(files)
}

fn parse_tests() -> Result<ItemImpl> {
    let mut item_impl: ItemImpl = parse_quote! {
        /// Constant tests.
        impl Test {}
    };
    let mut examples_arr: ExprArray = parse_quote!([]);
    let mut wat_files_arr: ExprArray = parse_quote!([]);
    let mut match_expr: ExprMatch = parse_quote! {
        match (module, name) {}
    };

    let mut push = |tests: &mut ExprArray, p: &PathBuf, bytes: &[u8]| {
        let name = p
            .with_extension("")
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            .to_string();

        let module = p
            .parent()
            .expect("parent not found for {p:?}")
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
            .to_string();

        let mut expr: ExprArray = parse_quote!([]);
        for byte in bytes {
            expr.elems.push(parse_quote!(#byte));
        }

        let ident: Ident = {
            let ident_name = module.to_uppercase() + "_" + &name.to_ascii_uppercase();
            let ident: Ident = Ident::new(&ident_name.replace('-', "_"), Span::call_site());
            let len = bytes.len();
            item_impl.items.push(parse_quote! {
                #[doc = concat!(" path: ", #module, "::", #name)]
                pub const #ident: [u8; #len] = #expr;
            });

            match_expr.arms.push(parse_quote! {
                (#module, #name) => Test {
                    module: module.into(),
                    name: name.into(),
                    wasm: Self::#ident.to_vec(),
                }
            });
            ident
        };

        tests.elems.push(parse_quote! {
            Test {
                module: #module.into(),
                name: #name.into(),
                wasm: Self::#ident.to_vec()
            }
        })
    };

    for wat in wat_files()? {
        let wat_bytes = fs::read(&wat)?;
        let wasm = wat::parse_bytes(&wat_bytes)?;
        push(&mut wat_files_arr, &wat, &wasm);
    }

    for example in examples()? {
        let wasm = fs::read(&example)?;
        push(&mut examples_arr, &example, &wasm);
    }

    match_expr.arms.push(parse_quote! {
        _ => return Err(anyhow::anyhow!("test not found: {{module: {}, name: {}}}", module, name))
    });

    let funcs: ItemImpl = parse_quote! {
        impl Test {
            /// Load test from module and name.
            pub fn load(module: &str, name: &str) -> anyhow::Result<Self> {
                Ok(#match_expr)
            }

            /// Example tests.
            pub fn examples() -> Vec<Test> {
                #examples_arr.to_vec()
            }

            /// Wat files tests.
            pub fn wat_files() -> Vec<Test> {
                #wat_files_arr.to_vec()
            }

            /// All tests.
            pub fn all() -> Vec<Test> {
                let mut tests = Self::examples();
                tests.extend(Self::wat_files());
                tests
            }
        }
    };

    item_impl.items.extend(funcs.items);
    Ok(item_impl)
}
