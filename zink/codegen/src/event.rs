use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens};
use syn::{
    parse::Parse, parse_quote, punctuated::Punctuated, spanned::Spanned, Arm, Data, DataEnum,
    DeriveInput, Expr, ExprMatch, Fields, FnArg, ImplItemFn, ItemFn, LitByteStr, Result, Token,
    Type, Variant, Visibility,
};

/// Expand the event interface with better error handling
pub fn parse(item: DeriveInput) -> TokenStream {
    let name = &item.ident;
    let name_str = name.to_string();
    let name_bytes = LitByteStr::new(name_str.as_bytes(), Span::call_site());

    if name_str.len() > 32 {
        panic!("Event name too long: {name_str}");
    }

    let Data::Enum(event_enum) = &item.data else {
        panic!("Event can only be derived for enums");
    };

    let mut expr_match: ExprMatch = parse_quote!(match self {});
    let variant_fns = event_enum
        .variants
        .iter()
        .map(|variant| impl_variant_fns(variant, &mut expr_match))
        .collect::<Vec<_>>();

    let variant_abis = event_enum
        .variants
        .iter()
        .map(|variant| abi_for_variant(&name_str, variant))
        .collect::<Vec<_>>();

    let combined_abi = format!("[{}]", variant_abis.join(","));
    let combined_abi_lit = proc_macro2::Literal::string(&combined_abi);

    quote! {
        impl #name {
            // Name of the event
            pub const fn name() -> &'static [u8] {
                #name_bytes
            }

            pub fn emit_name() {
                unsafe { zink::asm::evm::log0(Self::name()) }
            }

            pub fn register_abi() {
                let abi = Self::abi();
                unsafe {
                    let ptr = abi.as_ptr() as u32;
                    let len = abi.len() as u32;
                    zink::asm::emit_abi(ptr, len);
                }
            }

            pub fn abi() -> &'static str {
                #combined_abi_lit
            }

            #(#variant_fns)*

            pub fn emit(self) {
                #expr_match
            }
        }
    }
    .into()
}

fn abi_for_variant(event_name: &str, variant: &Variant) -> String {
    let variant_name = variant.ident.to_string();
    let mut params = Vec::new();
    let mut indexed_count = 0;

    for (index, field) in variant.fields.iter().enumerate() {
        let param_name = field
            .ident
            .clone()
            .unwrap_or(Ident::new(&format!("param_{index}"), Span::call_site()));

        let type_str = get_solidity_type(&field.ty)
            .unwrap_or_else(|e| panic!("Unsupported type for {}: {}", param_name, e));

        let is_indexed = field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("indexed"));
        if is_indexed {
            indexed_count += 1;
            if indexed_count > 3
                && !variant
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("anonymous"))
            {
                panic!(
                    "Event '{}' exceeds 3 indexed parameters for non-anonymous event",
                    variant_name
                );
            }
            if indexed_count > 4 {
                panic!(
                    "Event '{}' exceeds 4 indexed parameters even for anonymous event",
                    variant_name
                );
            }
        }
        params.push(format!(
            r#"{{"name":"{}","type":"{}","indexed":{}}}"#,
            param_name, type_str, is_indexed
        ));
    }

    let is_anonymous = variant
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("anonymous"));
    format!(
        r#"{{"type":"event","name":"{}","inputs":[{}],"anonymous":{}}}"#,
        event_name,
        params.join(","),
        is_anonymous
    )
}

fn get_solidity_type(ty: &Type) -> Result<String> {
    match ty {
        Type::Path(type_path) => {
            let segment = type_path.path.segments.last().ok_or_else(|| {
                syn::Error::new(ty.span(), "Invalid type path for event parameter")
            })?;
            let ident = &segment.ident;

            match ident.to_string().as_str() {
                "Address" => Ok("address".to_string()),
                "U256" => Ok("uint256".to_string()),
                "I256" => Ok("int256".to_string()),
                "Bytes32" => Ok("bytes32".to_string()),
                "bool" => Ok("bool".to_string()),
                "String" => Ok("string".to_string()),
                "Bytes" => Ok("bytes".to_string()),
                "Vec" => {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                            let inner_type = get_solidity_type(inner_ty)?;
                            Ok(format!("{}[]", inner_type))
                        } else {
                            Err(syn::Error::new(ty.span(), "Vec requires a type argument"))
                        }
                    } else {
                        Err(syn::Error::new(ty.span(), "Vec requires a type argument"))
                    }
                }
                _ => Err(syn::Error::new(
                    ty.span(),
                    "Unsupported type for event parameter",
                )),
            }
        }
        _ => Err(syn::Error::new(
            ty.span(),
            "Unsupported type for event parameter",
        )),
    }
}

fn impl_variant_fns(variant: &Variant, expr_match: &mut ExprMatch) -> ImplItemFn {
    let name = &variant.ident;
    let topic = variant.fields.len();

    // Parse function inputs
    let mut inputs: Punctuated<FnArg, Token![,]> = Punctuated::new();
    let mut args: Vec<Ident> = Vec::new();
    let mut fields: Vec<proc_macro2::TokenStream> = Vec::new();

    for (index, field) in variant.fields.iter().enumerate() {
        let var = field
            .ident
            .clone()
            .unwrap_or(Ident::new(&format!("param_{index}"), Span::call_site()));
        let ty = &field.ty;

        args.push(var.clone());
        inputs.push(FnArg::Typed(parse_quote!(#var: #ty)));
        fields.push(quote!(#var));
    }

    let name_snake = Ident::new(&name.to_string().to_snake_case(), Span::call_site());

    let arm = match &variant.fields {
        Fields::Named(_) => parse_quote! {
            Self::#name { #(#args),* } => Self::#name_snake(#(#args),*),
        },
        Fields::Unnamed(_) => parse_quote! {
            Self::#name(#(#args),*) => Self::#name_snake(#(#args),*),
        },
        Fields::Unit => parse_quote! {
            Self::#name => Self::#name_snake(),
        },
    };
    expr_match.arms.push(arm);

    let logn = Ident::new(&format!("log{topic}"), Span::call_site());
    let args = args
        .iter()
        .map(|arg| quote!(#arg.to_bytes32()))
        .collect::<Vec<_>>();

    parse_quote! {
        pub fn #name_snake(#inputs) {
            unsafe { zink::asm::evm::#logn(#(#args),*, &Self::name()) }
        }
    }
}
