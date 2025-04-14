use crate::utils::Bytes32;
use heck::AsSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::{Literal, Span, TokenTree};
use quote::quote;
use std::{cell::RefCell, collections::HashSet};
use syn::{
    meta::{self, ParseNestedMeta},
    parse::{Parse, ParseStream, Result},
    parse_quote, Attribute, Ident, ItemFn, ItemStruct, Visibility,
};

thread_local! {
   static STORAGE_REGISTRY: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
   static TRANSIENT_STORAGE_REGISTRY: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Storage type (persistent or transient)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    Persistent,
    Transient,
}

/// Storage attributes parser
pub struct Storage {
    /// Storage kind (persistent or transient)
    kind: StorageKind,
    /// kind of the storage
    ty: StorageType,
    /// The source and the target storage struct
    target: ItemStruct,
    /// Getter function of storage
    getter: Option<Ident>,
}

impl Storage {
    /// Parse from proc_macro attribute for persistent storage
    pub fn parse(ty: StorageType, target: ItemStruct) -> TokenStream {
        let storage = Self::from_parts(StorageKind::Persistent, ty, target);
        storage.expand()
    }

    /// Parse from proc_macro attribute for transient storage
    pub fn parse_transient(ty: StorageType, target: ItemStruct) -> TokenStream {
        let storage = Self::from_parts(StorageKind::Transient, ty, target);
        storage.expand()
    }

    fn from_parts(kind: StorageKind, ty: StorageType, target: ItemStruct) -> Self {
        let mut this = Self {
            kind,
            ty,
            target,
            getter: None,
        };

        let mut attrs: Vec<Attribute> = Default::default();
        for attr in this.target.attrs.iter().cloned() {
            if !attr.path().is_ident("getter") {
                attrs.push(attr);
                continue;
            }

            let Ok(list) = attr.meta.require_list().clone() else {
                panic!("Invalid getter arguments");
            };

            let Some(TokenTree::Ident(getter)) = list.tokens.clone().into_iter().nth(0) else {
                panic!("Invalid getter function name");
            };

            this.getter = Some(getter);
        }

        this.target.attrs = attrs;
        this
    }

    fn expand(mut self) -> TokenStream {
        match &self.ty {
            StorageType::Value(value) => self.expand_value(value.clone()),
            StorageType::Mapping { key, value } => self.expand_mapping(key.clone(), value.clone()),
            StorageType::DoubleKeyMapping { key1, key2, value } => {
                self.expand_dk_mapping(key1.clone(), key2.clone(), value.clone())
            }
            StorageType::Invalid => panic!("Invalid storage type"),
        }
    }

    fn expand_value(&mut self, value: Ident) -> TokenStream {
        let is = &self.target;
        let name = self.target.ident.clone();
        let slot = self.get_storage_slot(name.to_string());
        let key = slot.to_bytes32();

        let keyl = Literal::byte_string(&key);
        let trait_path = match self.kind {
            StorageKind::Persistent => quote!(zink::storage::Storage),
            StorageKind::Transient => quote!(zink::storage::TransientStorage),
        };

        let mut expanded = quote! {
            #is

            impl #trait_path for #name {
                #[cfg(not(target_family = "wasm"))]
                const STORAGE_KEY: [u8; 32] = *#keyl;
                const STORAGE_SLOT: i32 = #slot;

                type Value = #value;
            }
        };

        if let Some(getter) = self.getter() {
            let gs: proc_macro2::TokenStream = parse_quote! {
                #[allow(missing_docs)]
                #[zink::external]
                pub fn #getter() -> #value {
                    #name::get()
                }
            };
            expanded.extend(gs);
        }

        expanded.into()
    }

    fn expand_mapping(&mut self, key: Ident, value: Ident) -> TokenStream {
        let is = &self.target;
        let name = self.target.ident.clone();
        let slot = self.get_storage_slot(name.to_string());

        let trait_path = match self.kind {
            StorageKind::Persistent => quote!(zink::storage::Mapping),
            StorageKind::Transient => quote!(zink::transient_storage::TransientMapping),
        };

        let mut expanded = quote! {
            #is

            impl #trait_path for #name {
                const STORAGE_SLOT: i32 = #slot;

                type Key = #key;
                type Value = #value;

                #[cfg(not(target_family = "wasm"))]
                fn storage_key(key: Self::Key) -> [u8; 32] {
                    use zink::Value;

                    let mut seed = [0; 64];
                    seed[..32].copy_from_slice(&key.bytes32());
                    seed[32..].copy_from_slice(&Self::STORAGE_SLOT.bytes32());
                    zink::keccak256(&seed)
                }
            }
        };

        if let Some(getter) = self.getter() {
            let gs: proc_macro2::TokenStream = parse_quote! {
                #[allow(missing_docs)]
                #[zink::external]
                pub fn #getter(key: #key) -> #value {
                    #name::get(key)
                }
            };
            expanded.extend(gs);
        }

        expanded.into()
    }

    fn expand_dk_mapping(&mut self, key1: Ident, key2: Ident, value: Ident) -> TokenStream {
        let is = &self.target;
        let name = self.target.ident.clone();
        let slot = self.get_storage_slot(name.to_string());

        let trait_path = match self.kind {
            StorageKind::Persistent => quote!(zink::storage::DoubleKeyMapping),
            StorageKind::Transient => quote!(zink::transient_storage::DoubleKeyTransientMapping),
        };

        let mut expanded = quote! {
            #is

            impl #trait_path for #name {
                const STORAGE_SLOT: i32 = #slot;

                type Key1 = #key1;
                type Key2 = #key2;
                type Value = #value;

                #[cfg(not(target_family = "wasm"))]
                fn storage_key(key1: Self::Key1, key2: Self::Key2) -> [u8; 32] {
                    use zink::Value;

                    let mut seed = [0; 64];
                    seed[..32].copy_from_slice(&key1.bytes32());
                    seed[32..].copy_from_slice(&Self::STORAGE_SLOT.bytes32());
                    let skey1 = zink::keccak256(&seed);
                    seed[..32].copy_from_slice(&skey1);
                    seed[32..].copy_from_slice(&key2.bytes32());
                    zink::keccak256(&seed)
                }
            }
        };

        if let Some(getter) = self.getter() {
            let gs: proc_macro2::TokenStream = parse_quote! {
                #[allow(missing_docs)]
                #[zink::external]
                pub fn #getter(key1: #key1, key2: #key2) -> #value {
                    #name::get(key1, key2)
                }
            };
            expanded.extend(gs);
        }

        expanded.into()
    }

    fn get_storage_slot(&self, name: String) -> i32 {
        match self.kind {
            StorageKind::Persistent => STORAGE_REGISTRY.with_borrow_mut(|r| {
                let key = r.len();
                if !r.insert(name.clone()) {
                    panic!("Storage {name} has already been declared");
                }
                key
            }) as i32,
            StorageKind::Transient => TRANSIENT_STORAGE_REGISTRY.with_borrow_mut(|r| {
                let key = r.len();
                if !r.insert(name.clone()) {
                    panic!("Transient storage {name} has already been declared");
                }
                key
            }) as i32,
        }
    }

    /// Get the getter of this storage
    fn getter(&mut self) -> Option<Ident> {
        let mut getter = if matches!(self.target.vis, Visibility::Public(_)) {
            let fname = Ident::new(
                &AsSnakeCase(self.target.ident.to_string()).to_string(),
                Span::call_site(),
            );
            Some(fname)
        } else {
            None
        };

        self.getter.take().or(getter)
    }
}

/// Zink storage type parser
#[derive(Default, Debug)]
pub enum StorageType {
    /// Single value storage
    Value(Ident),
    /// Mapping storage
    Mapping { key: Ident, value: Ident },
    /// Double key mapping storage
    DoubleKeyMapping {
        key1: Ident,
        key2: Ident,
        value: Ident,
    },
    /// Invalid storage type
    #[default]
    Invalid,
}

impl From<TokenStream> for StorageType {
    fn from(input: TokenStream) -> Self {
        let tokens = input.to_string();
        let types: Vec<_> = tokens.split(',').collect();
        match types.len() {
            1 => StorageType::Value(Ident::new(types[0].trim(), Span::call_site())),
            2 => StorageType::Mapping {
                key: Ident::new(types[0].trim(), Span::call_site()),
                value: Ident::new(types[1].trim(), Span::call_site()),
            },
            3 => StorageType::DoubleKeyMapping {
                key1: Ident::new(types[0].trim(), Span::call_site()),
                key2: Ident::new(types[1].trim(), Span::call_site()),
                value: Ident::new(types[2].trim(), Span::call_site()),
            },
            _ => panic!("Invalid storage attributes"),
        }
    }
}
